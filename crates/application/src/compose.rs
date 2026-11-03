//! Cas d'usage de la rédaction (phase 7) : brouillons, préparation d'une
//! réponse, mise en file, envoi SMTP, copies dans « Envoyés » et
//! « Brouillons ».
//!
//! Les règles (destinataires, sujets, fil, validation, états de la file,
//! politique de copie) sont dans `norelly_domain::compose` ; ce module
//! orchestre les ports. Les étapes qui demandent une session IMAP
//! ([`FileSentCopies`], [`SyncDrafts`]) sont regroupées par
//! [`crate::compose_worker`], qui n'ouvre une connexion que s'il y a du
//! travail.

use norelly_domain::account::AccountId;
use norelly_domain::address::EmailAddress;
use norelly_domain::compose::draft::{ComposeError, Draft, Recipient, remote_drafts_supported};
use norelly_domain::compose::outbox::{OutboxState, SendFailure, sent_copy_needed};
use norelly_domain::compose::{
    ReplyKind, Threading, forward_subject, quote, reply_recipients, reply_subject, reply_threading,
};
use norelly_domain::connection::ConnectionError;
use norelly_domain::mailbox::{Correspondent, MailboxName, MailboxRole};
use norelly_domain::ports::{
    BlobStore, ComposeBuildError, ComposedAttachment, Composition, ConnectRequest, DraftId,
    DraftStore, MailSender, MessageComposer, MessageId, MessageLocator, OutboxId, OutboxStore,
    PendingOpStore, RemoteMailboxSession, StoreError, StoredBody, StoredDraft,
};

/// Échec d'un cas d'usage de rédaction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComposeFailure {
    #[error(transparent)]
    Invalid(#[from] ComposeError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Build(#[from] ComposeBuildError),
    /// Message, brouillon ou pièce jointe introuvable.
    #[error("introuvable")]
    NotFound,
}

/// Cas d'usage : enregistrer un brouillon (même incomplet).
#[derive(Debug)]
pub struct SaveDraft<'a, D> {
    pub drafts: &'a D,
}

impl<D: DraftStore> SaveDraft<'_, D> {
    pub async fn execute(
        &self,
        id: Option<DraftId>,
        draft: &Draft,
        now: i64,
    ) -> Result<StoredDraft, ComposeFailure> {
        draft.check_storable()?;
        Ok(self.drafts.save_draft(id, draft, now).await?)
    }
}

/// Cas d'usage : mettre un message en file d'envoi (le brouillon `id`
/// éventuel est retiré dans la même transaction).
#[derive(Debug)]
pub struct QueueMessage<'a, S, B, M> {
    pub store: &'a S,
    pub blobs: &'a B,
    pub composer: &'a M,
}

impl<S, B, M> QueueMessage<'_, S, B, M>
where
    S: DraftStore + OutboxStore,
    B: BlobStore,
    M: MessageComposer,
{
    pub async fn execute(
        &self,
        id: Option<DraftId>,
        draft: &Draft,
        from: &Recipient,
        now: i64,
    ) -> Result<OutboxId, ComposeFailure> {
        draft.check_sendable()?;
        let mut attachments = Vec::with_capacity(draft.attachments.len());
        for a in &draft.attachments {
            let bytes = self
                .blobs
                .get(draft.account, &a.blob)
                .await?
                .ok_or(ComposeFailure::NotFound)?;
            attachments.push(ComposedAttachment {
                file_name: a.file_name.clone(),
                content_type: a.content_type.clone(),
                bytes,
            });
        }
        let composition = Composition {
            from: from.clone(),
            draft: draft.clone(),
            attachments,
            date: now,
            message_id: self.composer.new_message_id(&from.address)?,
        };
        let built = self.composer.compose(&composition)?;
        Ok(self
            .store
            .enqueue(
                draft.account,
                &built.outgoing,
                &built.local_copy,
                &draft.subject,
                id,
                now,
            )
            .await?)
    }
}

/// Cas d'usage : préparer une réponse, une réponse à tous ou un transfert.
#[derive(Debug)]
pub struct PrepareReply<'a, L, B, M> {
    pub locator: &'a L,
    pub blobs: &'a B,
    pub composer: &'a M,
}

impl<L, B, M> PrepareReply<'_, L, B, M>
where
    L: MessageLocator,
    B: BlobStore,
    M: MessageComposer,
{
    /// `own` : adresses de l'utilisateur ; `attribution` : en-tête de
    /// citation (ou de transfert), construit par l'interface dans la langue
    /// de l'utilisateur (ADR-011).
    pub async fn execute(
        &self,
        message: MessageId,
        kind: ReplyKind,
        own: &[EmailAddress],
        attribution: &str,
    ) -> Result<Draft, ComposeFailure> {
        let location = self
            .locator
            .locate(message)
            .await?
            .ok_or(ComposeFailure::NotFound)?;
        // Message complet, sinon ses en-têtes (corps partiel : la citation
        // reste vide).
        let blob = match &location.body {
            StoredBody::Full(blob) => blob,
            StoredBody::Partial { header, .. } => header,
            StoredBody::Missing => return Err(ComposeFailure::NotFound),
        };
        let raw = self
            .blobs
            .get(location.account, blob)
            .await?
            .ok_or(ComposeFailure::NotFound)?;
        let source = self
            .composer
            .reply_source(&raw)
            .ok_or(ComposeFailure::NotFound)?;
        let recipients = reply_recipients(&source, kind, own);
        let (subject, threading) = match kind {
            ReplyKind::Forward => (forward_subject(&source.subject), Threading::default()),
            ReplyKind::Reply | ReplyKind::ReplyAll => {
                (reply_subject(&source.subject), reply_threading(&source))
            }
        };
        let mut draft = Draft::empty(location.account);
        draft.to = valid_recipients(&recipients.to);
        draft.cc = valid_recipients(&recipients.cc);
        draft.subject = subject;
        draft.text = format!("\n\n{}", quote(&source.body, attribution));
        draft.html = Some(format!(
            "<p></p>{}",
            self.composer.quote_html(&source.body, attribution)
        ));
        draft.threading = threading;
        Ok(draft)
    }
}

/// Bilan d'un passage de la file d'envoi.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeliveryReport {
    /// Messages acceptés par le serveur.
    pub sent: Vec<OutboxId>,
    /// Essais en échec (nouvelle tentative ou échec définitif).
    pub failed: Vec<(OutboxId, SendFailure)>,
    /// Prochain essai prévu (secondes Unix).
    pub next_attempt: Option<i64>,
}

/// Cas d'usage : envoyer les messages dus de la file d'un compte.
#[derive(Debug)]
pub struct DeliverOutbox<'a, O, S> {
    pub account: AccountId,
    pub outbox: &'a O,
    pub sender: &'a S,
}

impl<O: OutboxStore, S: MailSender> DeliverOutbox<'_, O, S> {
    pub async fn execute(
        &self,
        request: &ConnectRequest,
        now: i64,
    ) -> Result<DeliveryReport, StoreError> {
        let mut report = DeliveryReport::default();
        for item in self.outbox.outbox(self.account).await? {
            if !item.entry.is_due(now) {
                continue;
            }
            let Ok(started) = item.entry.start() else {
                continue;
            };
            // Réservé avant l'envoi : jamais deux envois du même message.
            if !self.outbox.transition(item.id, item.entry, started).await? {
                continue;
            }
            let result = self.sender.send(request, &item.message).await;
            let next = match &result {
                Ok(()) => started.succeeded(),
                Err(e) => started.failed(SendFailure::of(e), now),
            };
            let Ok(next) = next else { continue };
            self.outbox.transition(item.id, started, next).await?;
            match result {
                Ok(()) => {
                    tracing::debug!(account = %self.account, "message envoyé");
                    report.sent.push(item.id);
                }
                Err(e) => {
                    let failure = SendFailure::of(&e);
                    tracing::warn!(account = %self.account, error = %e, ?failure, "envoi en échec");
                    report.failed.push((item.id, failure));
                    // Hors ligne : inutile de tenter les suivants.
                    if failure == SendFailure::Unreachable {
                        break;
                    }
                }
            }
        }
        report.next_attempt = self
            .outbox
            .outbox(self.account)
            .await?
            .iter()
            .filter_map(|i| match i.entry.state {
                OutboxState::Queued { not_before } => Some(not_before),
                _ => None,
            })
            .min();
        Ok(report)
    }
}

/// Cas d'usage, au démarrage : un envoi resté « en cours » est incertain.
#[derive(Debug)]
pub struct RecoverOutbox<'a, O> {
    pub account: AccountId,
    pub outbox: &'a O,
}

impl<O: OutboxStore> RecoverOutbox<'_, O> {
    pub async fn execute(&self) -> Result<usize, StoreError> {
        let mut recovered = 0;
        for item in self.outbox.outbox(self.account).await? {
            let next = item.entry.recovered();
            if next != item.entry && self.outbox.transition(item.id, item.entry, next).await? {
                tracing::warn!(account = %self.account, "envoi interrompu : issue incertaine");
                recovered += 1;
            }
        }
        Ok(recovered)
    }
}

/// Échec d'une étape IMAP de la rédaction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RemoteComposeFailure {
    #[error(transparent)]
    Connection(#[from] ConnectionError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Cas d'usage : déposer les copies des messages envoyés dans « Envoyés »
/// (sauf serveur qui le fait lui-même) puis les retirer de la file.
#[derive(Debug)]
pub struct FileSentCopies<'a, O, F> {
    pub account: AccountId,
    pub outbox: &'a O,
    /// Dossiers du compte (rôle `\Sent`).
    pub folders: &'a F,
}

impl<O: OutboxStore, F: PendingOpStore> FileSentCopies<'_, O, F> {
    pub async fn execute<R: RemoteMailboxSession>(
        &self,
        session: &mut R,
    ) -> Result<usize, RemoteComposeFailure> {
        let sent: Vec<_> = self
            .outbox
            .outbox(self.account)
            .await?
            .into_iter()
            .filter(|i| i.entry.state == OutboxState::Sent)
            .collect();
        if sent.is_empty() {
            return Ok(0);
        }
        let target = if sent_copy_needed(session.capabilities()) {
            folder_with_role(self.folders, self.account, MailboxRole::Sent).await?
        } else {
            None
        };
        let mut filed = 0;
        for item in sent {
            if let Some(name) = &target {
                match session.append(name, &item.sent_copy, false).await {
                    Ok(_) => filed += 1,
                    // Refus (quota…) : la copie est abandonnée, le message
                    // est bien parti.
                    Err(ConnectionError::Rejected(reason)) => {
                        tracing::warn!(account = %self.account, %reason, "copie dans Envoyés refusée");
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            self.outbox.remove(item.id).await?;
        }
        Ok(filed)
    }
}

/// Bilan d'une synchronisation des brouillons.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftSyncReport {
    /// Brouillons copiés sur le serveur.
    pub saved: usize,
    /// Anciennes copies supprimées du serveur (ou oubliées).
    pub removed: usize,
}

/// Cas d'usage : copier les brouillons modifiés dans « Brouillons »
/// (remplacement de l'ancienne copie, UIDPLUS) et supprimer les copies
/// périmées.
#[derive(Debug)]
pub struct SyncDrafts<'a, D, F, B, M> {
    pub account: AccountId,
    pub drafts: &'a D,
    pub folders: &'a F,
    pub blobs: &'a B,
    pub composer: &'a M,
}

impl<D, F, B, M> SyncDrafts<'_, D, F, B, M>
where
    D: DraftStore,
    F: PendingOpStore,
    B: BlobStore,
    M: MessageComposer,
{
    pub async fn execute<R: RemoteMailboxSession>(
        &self,
        session: &mut R,
        from: &Recipient,
        now: i64,
    ) -> Result<DraftSyncReport, RemoteComposeFailure> {
        let mut report = DraftSyncReport::default();
        if !remote_drafts_supported(session.capabilities()) {
            return Ok(report);
        }
        let Some(mailbox) =
            folder_with_role(self.folders, self.account, MailboxRole::Drafts).await?
        else {
            return Ok(report);
        };
        for stored in self.drafts.drafts(self.account).await? {
            if !stored.sync.remote_save_due(now) {
                continue;
            }
            let raw = match self.local_copy(&stored.draft, from, now).await {
                Ok(raw) => raw,
                Err(e) => {
                    tracing::warn!(account = %self.account, error = %e, "brouillon non copié");
                    continue;
                }
            };
            let copy = session.append(&mailbox, &raw, true).await?;
            let (next, old) = stored.sync.remote_saved(stored.sync.revision, copy, now);
            self.drafts
                .set_remote(self.account, stored.id, next, old)
                .await?;
            report.saved += 1;
        }
        let trash = self.drafts.remote_trash(self.account).await?;
        if trash.is_empty() {
            return Ok(report);
        }
        let selected = session.select_writable(&mailbox).await?;
        for copy in trash {
            // UIDVALIDITY changée : l'UID désignerait un autre message.
            // Message sans \Draft : plus notre brouillon. Dans les deux
            // cas, rien n'est supprimé.
            if selected.uid_validity == Some(copy.validity)
                && session
                    .message_flags(copy.uid)
                    .await?
                    .is_some_and(|f| f.draft)
            {
                session.mark_deleted(copy.uid).await?;
                session.expunge_uid(copy.uid).await?;
            }
            self.drafts.forget_remote(self.account, copy).await?;
            report.removed += 1;
        }
        Ok(report)
    }
}

impl<D, F, B, M> SyncDrafts<'_, D, F, B, M>
where
    D: DraftStore,
    F: PendingOpStore,
    B: BlobStore,
    M: MessageComposer,
{
    /// Copie locale (avec Cci et pièces jointes) d'un brouillon.
    async fn local_copy(
        &self,
        draft: &Draft,
        from: &Recipient,
        now: i64,
    ) -> Result<Vec<u8>, ComposeFailure> {
        let mut attachments = Vec::with_capacity(draft.attachments.len());
        for a in &draft.attachments {
            let bytes = self
                .blobs
                .get(draft.account, &a.blob)
                .await?
                .ok_or(ComposeFailure::NotFound)?;
            attachments.push(ComposedAttachment {
                file_name: a.file_name.clone(),
                content_type: a.content_type.clone(),
                bytes,
            });
        }
        let composition = Composition {
            from: from.clone(),
            draft: draft.clone(),
            attachments,
            date: now,
            message_id: self.composer.new_message_id(&from.address)?,
        };
        Ok(self.composer.compose(&composition)?.local_copy)
    }
}

/// Nom du dossier d'un compte ayant ce rôle.
async fn folder_with_role<F: PendingOpStore>(
    folders: &F,
    account: AccountId,
    role: MailboxRole,
) -> Result<Option<MailboxName>, StoreError> {
    Ok(folders
        .action_folders(account)
        .await?
        .into_iter()
        .find(|f| f.role == role && f.selectable)
        .map(|f| f.name))
}

/// Destinataires proposés, revalidés (une adresse invalide est écartée).
fn valid_recipients(list: &[Correspondent]) -> Vec<Recipient> {
    list.iter()
        .filter_map(|c| {
            EmailAddress::parse(&c.address)
                .ok()
                .map(|a| Recipient::new(&c.name, a))
        })
        .collect()
}

#[cfg(test)]
mod tests;
