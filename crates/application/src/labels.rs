//! Cas d'usage des étiquettes : liste d'un compte ([`ListLabels`]),
//! création ([`CreateLabel`]), couleur ([`SetLabelColor`]), renommage
//! ([`RenameLabel`]) et suppression ([`DeleteLabel`]), réécriture sur le
//! serveur ([`ReplayRewrites`]), menu « Étiqueter… » d'une sélection
//! ([`LabelMenuFor`]). Poser ou retirer une étiquette est une action
//! ([`crate::actions::PerformAction`]).
//!
//! Les règles (nom, encodage, doublons, liste, portée, mots-clés
//! permanents, lots) sont dans `norelly_domain::labels`.

use std::collections::{BTreeMap, BTreeSet};

use norelly_domain::account::AccountId;
use norelly_domain::connection::ConnectionError;
use norelly_domain::conversation::ConversationScope;
use norelly_domain::labels::{
    KeywordPermission, Label, LabelColor, LabelError, LabelMenu, LabelName, LabelRewrite,
    MAX_REWRITE_ATTEMPTS, StoredLabel, account_labels, hidden_keywords, label_menu, label_named,
    plan_create, plan_rename, rewrite_batches,
};
use norelly_domain::mailbox::{Keyword, MailboxId, MailboxName, classify_mailboxes};
use norelly_domain::ports::{
    ConversationId, LabelStore, PendingOpStore, RemoteMailboxSession, StoreError,
};

use crate::actions::ReplayFailure;

/// Échec d'un cas d'usage des étiquettes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LabelFailure {
    #[error(transparent)]
    Invalid(#[from] LabelError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Cas d'usage : étiquettes d'un compte.
#[derive(Debug)]
pub struct ListLabels<'a, S> {
    pub store: &'a S,
}

impl<S: LabelStore> ListLabels<'_, S> {
    /// Étiquettes du compte (domaine : [`account_labels`]).
    pub async fn execute(&self, account: AccountId) -> Result<Vec<Label>, StoreError> {
        let stored = self.store.stored_labels(account).await?;
        let seen = self.store.seen_keywords(account).await?;
        let rewrites = self.rewrites(account).await?;
        Ok(account_labels(&stored, &seen, &rewrites))
    }

    /// Mots-clés jamais affichés sur le compte (domaine :
    /// [`hidden_keywords`]).
    pub async fn hidden(&self, account: AccountId) -> Result<BTreeSet<Keyword>, StoreError> {
        let stored = self.store.stored_labels(account).await?;
        let rewrites = self.rewrites(account).await?;
        Ok(hidden_keywords(&stored, &rewrites))
    }

    async fn rewrites(&self, account: AccountId) -> Result<Vec<LabelRewrite>, StoreError> {
        Ok(self
            .store
            .label_rewrites(account)
            .await?
            .into_iter()
            .map(|(_, r)| r)
            .collect())
    }
}

/// L'étiquette `keyword` est listée sur le compte.
async fn listed<S: LabelStore>(
    store: &S,
    account: AccountId,
    keyword: &Keyword,
) -> Result<Option<Label>, StoreError> {
    Ok(ListLabels { store }
        .execute(account)
        .await?
        .into_iter()
        .find(|l| &l.keyword == keyword))
}

/// Cas d'usage : créer une étiquette (réglages, ou menu « Étiqueter… »).
/// Une étiquette supprimée de la liste puis créée de nouveau sous le même
/// nom réapparaît.
#[derive(Debug)]
pub struct CreateLabel<'a, S> {
    pub store: &'a S,
}

impl<S: LabelStore> CreateLabel<'_, S> {
    pub async fn execute(
        &self,
        account: AccountId,
        name: &LabelName,
        color: Option<LabelColor>,
    ) -> Result<Keyword, LabelFailure> {
        let labels = ListLabels { store: self.store }.execute(account).await?;
        let keyword = plan_create(name, &labels)?;
        self.store
            .save_label(
                account,
                &StoredLabel {
                    keyword: keyword.clone(),
                    color,
                    hidden: false,
                },
            )
            .await?;
        Ok(keyword)
    }
}

impl<S: LabelStore> CreateLabel<'_, S> {
    /// Étiquette `name` du compte : celle qui porte déjà ce nom, sinon une
    /// nouvelle (menu « Étiqueter… »).
    pub async fn existing_or_new(
        &self,
        account: AccountId,
        name: &LabelName,
    ) -> Result<Keyword, LabelFailure> {
        let labels = ListLabels { store: self.store }.execute(account).await?;
        if let Some(existing) = label_named(name, &labels) {
            return Ok(existing.keyword.clone());
        }
        self.execute(account, name, None).await
    }
}

/// Cas d'usage : couleur d'une étiquette listée (`None` : sans couleur).
#[derive(Debug)]
pub struct SetLabelColor<'a, S> {
    pub store: &'a S,
}

impl<S: LabelStore> SetLabelColor<'_, S> {
    pub async fn execute(
        &self,
        account: AccountId,
        keyword: &Keyword,
        color: Option<LabelColor>,
    ) -> Result<(), LabelFailure> {
        if listed(self.store, account, keyword).await?.is_none() {
            return Err(LabelError::Unknown.into());
        }
        self.store
            .save_label(
                account,
                &StoredLabel {
                    keyword: keyword.clone(),
                    color,
                    hidden: false,
                },
            )
            .await?;
        Ok(())
    }
}

/// Cas d'usage : renommer une étiquette. Le renommage est appliqué tout de
/// suite en local et enregistré ; [`ReplayRewrites`] réécrit ensuite le
/// mot-clé sur tous les messages du serveur. Renvoie le nouveau mot-clé.
#[derive(Debug)]
pub struct RenameLabel<'a, S> {
    pub store: &'a S,
}

impl<S: LabelStore> RenameLabel<'_, S> {
    pub async fn execute(
        &self,
        account: AccountId,
        keyword: &Keyword,
        name: &LabelName,
    ) -> Result<Keyword, LabelFailure> {
        let labels = ListLabels { store: self.store }.execute(account).await?;
        let rewrite = plan_rename(keyword, name, &labels)?;
        self.store.record_rewrite(account, &rewrite).await?;
        rewrite.to.ok_or_else(|| LabelError::Unknown.into())
    }
}

/// Cas d'usage : supprimer une étiquette de la liste ; avec
/// `remove_from_messages`, la retirer aussi de tous les messages (comme un
/// renommage, appliqué en local puis rejoué sur le serveur).
#[derive(Debug)]
pub struct DeleteLabel<'a, S> {
    pub store: &'a S,
}

impl<S: LabelStore> DeleteLabel<'_, S> {
    pub async fn execute(
        &self,
        account: AccountId,
        keyword: &Keyword,
        remove_from_messages: bool,
    ) -> Result<(), LabelFailure> {
        if listed(self.store, account, keyword).await?.is_none() {
            return Err(LabelError::Unknown.into());
        }
        if remove_from_messages {
            let rewrite = LabelRewrite {
                from: keyword.clone(),
                to: None,
            };
            self.store.record_rewrite(account, &rewrite).await?;
        } else {
            self.store
                .save_label(
                    account,
                    &StoredLabel {
                        keyword: keyword.clone(),
                        color: None,
                        hidden: true,
                    },
                )
                .await?;
        }
        Ok(())
    }
}

/// Bilan d'un rejeu des réécritures.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewriteReport {
    /// Réécritures terminées.
    pub completed: usize,
    /// Réécritures abandonnées après trop d'essais.
    pub abandoned: usize,
    /// Boîtes dont des messages ont changé (à resynchroniser).
    pub touched: BTreeSet<MailboxName>,
}

/// Cas d'usage : rejouer les réécritures en attente d'un compte sur une
/// session ouverte, dossier par dossier : `UID SEARCH KEYWORD` de l'ancien
/// mot-clé, puis, par lots d'UID, le nouveau posé avant que l'ancien soit
/// retiré. Une interruption laisse un état cohérent (un message porte
/// l'ancien mot-clé, ou les deux, jamais aucun) ; la réécriture, gardée,
/// reprend à la session suivante, en cherchant de nouveau les messages qui
/// portent encore l'ancien mot-clé. Un dossier qui refuse le nouveau
/// mot-clé est laissé tel quel ; une réécriture qui échoue
/// [`MAX_REWRITE_ATTEMPTS`] fois est abandonnée.
#[derive(Debug)]
pub struct ReplayRewrites<'a, S> {
    pub account: AccountId,
    pub store: &'a S,
}

impl<S: LabelStore> ReplayRewrites<'_, S> {
    pub async fn execute<R: RemoteMailboxSession>(
        &self,
        session: &mut R,
    ) -> Result<RewriteReport, ReplayFailure> {
        let mut report = RewriteReport::default();
        let pending = self.store.label_rewrites(self.account).await?;
        if pending.is_empty() {
            return Ok(report);
        }
        // Chaque essai est compté avant tout envoi ; au-delà du plafond, la
        // réécriture est abandonnée.
        let mut active = Vec::new();
        for (id, rewrite) in pending {
            let Some(attempts) = self.store.claim_rewrite(self.account, id).await? else {
                continue;
            };
            if attempts > MAX_REWRITE_ATTEMPTS {
                tracing::warn!(account = %self.account, attempts, "réécriture d'étiquette abandonnée après trop d'essais");
                self.store.complete_rewrite(self.account, id).await?;
                report.abandoned += 1;
            } else {
                active.push((id, rewrite));
            }
        }
        let mailboxes: Vec<MailboxName> = classify_mailboxes(session.list_mailboxes().await?)
            .into_iter()
            .filter(|m| m.selectable)
            .map(|m| m.name)
            .collect();
        // Abandon : la copie locale, réécrite d'avance, est relue du
        // serveur dans tous les dossiers.
        if report.abandoned > 0 {
            for mailbox in &mailboxes {
                self.resync(mailbox, &mut report).await?;
            }
        }
        if active.is_empty() {
            return Ok(report);
        }
        let rewrites: Vec<LabelRewrite> = active.iter().map(|(_, r)| r.clone()).collect();
        for mailbox in &mailboxes {
            let done = self.rewrite_mailbox(session, mailbox, &rewrites).await?;
            if done.changed {
                report.touched.insert(mailbox.clone());
            }
            if done.left_behind {
                self.resync(mailbox, &mut report).await?;
            }
        }
        for (id, _) in active {
            self.store.complete_rewrite(self.account, id).await?;
            report.completed += 1;
        }
        Ok(report)
    }

    /// Dossier où une réécriture n'a pas pu se faire : ses drapeaux seront
    /// relus entièrement à la synchronisation qui suit.
    async fn resync(
        &self,
        mailbox: &MailboxName,
        report: &mut RewriteReport,
    ) -> Result<(), StoreError> {
        self.store.forget_flag_state(self.account, mailbox).await?;
        report.touched.insert(mailbox.clone());
        Ok(())
    }

    /// Réécrit un dossier (un seul `SELECT` pour toutes les réécritures,
    /// dans leur ordre). Un refus du serveur (`NO`), ou un dossier qui
    /// refuse le nouveau mot-clé, laisse le dossier tel quel.
    async fn rewrite_mailbox<R: RemoteMailboxSession>(
        &self,
        session: &mut R,
        mailbox: &MailboxName,
        rewrites: &[LabelRewrite],
    ) -> Result<MailboxRewrite, ReplayFailure> {
        let mut done = MailboxRewrite::default();
        let selected = match session.select_writable(mailbox).await {
            Ok(selected) => selected,
            Err(ConnectionError::Rejected(reason)) => {
                tracing::warn!(account = %self.account, %reason, "dossier non sélectionnable pour une réécriture d'étiquette");
                done.left_behind = true;
                return Ok(done);
            }
            Err(e) => return Err(e.into()),
        };
        if selected.keywords != KeywordPermission::Unknown {
            self.store
                .set_keyword_permission(self.account, mailbox, &selected.keywords)
                .await?;
        }
        for rewrite in rewrites {
            if !rewrite.applies_to(&selected.keywords) {
                tracing::info!("dossier qui refuse le nouveau mot-clé : laissé tel quel");
                done.left_behind = true;
                continue;
            }
            match rewrite_keyword(session, rewrite).await {
                Ok(changed) => done.changed |= changed,
                Err(ConnectionError::Rejected(reason)) => {
                    tracing::warn!(account = %self.account, %reason, "réécriture d'étiquette refusée dans un dossier");
                    done.left_behind = true;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(done)
    }
}

/// Bilan de la réécriture d'un dossier.
#[derive(Debug, Default)]
struct MailboxRewrite {
    /// Des messages ont changé.
    changed: bool,
    /// Une réécriture n'a pas pu s'y faire (refus) : la copie locale est à
    /// relire.
    left_behind: bool,
}

/// `UID SEARCH KEYWORD`, puis, par lots, le nouveau mot-clé posé avant que
/// l'ancien soit retiré ; `true` si des messages le portaient.
async fn rewrite_keyword<R: RemoteMailboxSession>(
    session: &mut R,
    rewrite: &LabelRewrite,
) -> Result<bool, ConnectionError> {
    let uids = session.search_keyword(&rewrite.from).await?;
    for batch in rewrite_batches(&uids) {
        if let Some(to) = &rewrite.to {
            session.store_keyword(&batch, to, true).await?;
        }
        session.store_keyword(&batch, &rewrite.from, false).await?;
    }
    Ok(!uids.is_empty())
}

/// Étiquettes proposées pour une sélection de conversations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionLabels {
    /// Menu décidé par le domaine ([`label_menu`]).
    pub menu: LabelMenu,
    /// Étiquettes des comptes concernés (noms, couleurs), sans doublon.
    pub labels: Vec<Label>,
    /// Comptes des conversations.
    pub accounts: Vec<AccountId>,
}

/// Cas d'usage : menu « Étiqueter… » des conversations `conversations`
/// affichées dans `scope` (éventuellement de plusieurs comptes, boîte
/// unifiée : leurs étiquettes sont réunies).
#[derive(Debug)]
pub struct LabelMenuFor<'a, S> {
    pub store: &'a S,
}

impl<S: LabelStore + PendingOpStore> LabelMenuFor<'_, S> {
    pub async fn execute(
        &self,
        conversations: &[ConversationId],
        scope: &ConversationScope,
    ) -> Result<SelectionLabels, StoreError> {
        let mut members = Vec::new();
        let mut accounts = BTreeSet::new();
        for id in conversations {
            let list = self.store.conversation_members(*id).await?;
            if let Some(first) = list.first()
                && let Some(target) = self.store.action_target(first.id).await?
            {
                accounts.insert(target.account);
            }
            members.push(list);
        }
        let mut labels: Vec<Label> = Vec::new();
        let mut permissions: BTreeMap<MailboxId, KeywordPermission> = BTreeMap::new();
        let list = ListLabels { store: self.store };
        for account in &accounts {
            for label in list.execute(*account).await? {
                if !labels.iter().any(|l| l.keyword == label.keyword) {
                    labels.push(label);
                }
            }
            permissions.extend(self.store.keyword_permissions(*account).await?);
        }
        let keywords: Vec<Keyword> = labels.iter().map(|l| l.keyword.clone()).collect();
        let menu = label_menu(&members, scope, &keywords, |mailbox| {
            permissions.get(&mailbox).cloned().unwrap_or_default()
        });
        Ok(SelectionLabels {
            menu,
            labels,
            accounts: accounts.into_iter().collect(),
        })
    }
}

#[cfg(test)]
mod tests;
