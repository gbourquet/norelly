//! Ordre des écritures dans un trousseau. Un appel au trousseau tourne dans
//! une tâche bloquante, qui continue même quand l'appelant l'abandonne
//! (délai dépassé, trousseau en attente de déverrouillage) : une écriture
//! abandonnée ne doit pas écraser, en aboutissant plus tard, une écriture
//! ou un effacement plus récent de la même clé.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// Numérote les écritures dans l'ordre des appels et n'applique pas une
/// écriture dépassée.
#[derive(Debug, Default)]
pub(crate) struct WriteOrder {
    next: AtomicU64,
    /// Par clé, numéro de la dernière écriture appliquée, sous un verrou
    /// propre à la clé : une écriture bloquée n'arrête que sa clé.
    applied: Mutex<HashMap<String, Arc<Mutex<u64>>>>,
}

impl WriteOrder {
    /// Numéro d'une écriture, pris au moment de l'appel (avant la tâche
    /// bloquante).
    pub(crate) fn ticket(&self) -> u64 {
        self.next.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Exécute `write` (bloquant) pour `key`, sauf si une écriture plus
    /// récente de cette clé a déjà été appliquée : rend alors `Ok(None)`.
    /// Les écritures d'une même clé sont exécutées une à la fois.
    pub(crate) fn run<T, E>(
        &self,
        key: &str,
        ticket: u64,
        write: impl FnOnce() -> Result<T, E>,
    ) -> Result<Option<T>, E> {
        let slot = Arc::clone(
            self.applied
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(key.to_owned())
                .or_default(),
        );
        let mut last = slot.lock().unwrap_or_else(PoisonError::into_inner);
        if *last > ticket {
            return Ok(None);
        }
        let result = write()?;
        *last = ticket;
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(order: &WriteOrder, value: &mut Vec<&'static str>, ticket: u64, v: &'static str) {
        let _ = order.run("k", ticket, || {
            value.push(v);
            Ok::<(), ()>(())
        });
    }

    #[test]
    fn an_abandoned_write_never_overwrites_a_newer_one() {
        let order = WriteOrder::default();
        let (old, new) = (order.ticket(), order.ticket());
        let mut written = Vec::new();
        // L'écriture récente aboutit d'abord, l'ancienne (abandonnée) après.
        write(&order, &mut written, new, "r3");
        write(&order, &mut written, old, "r2");
        assert_eq!(written, vec!["r3"]);
    }

    #[test]
    fn a_stuck_write_does_not_block_other_keys() {
        let order = std::sync::Arc::new(WriteOrder::default());
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let stuck = std::sync::Arc::clone(&order);
        let ticket = order.ticket();
        let a = std::thread::spawn(move || {
            stuck.run("a", ticket, || {
                let _ = entered_tx.send(());
                release_rx.recv().map_err(|_| ())
            })
        });
        let _ = entered_rx.recv();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let other = std::sync::Arc::clone(&order);
        let ticket = order.ticket();
        std::thread::spawn(move || {
            let _ = done_tx.send(other.run("b", ticket, || Ok::<_, ()>(())));
        });
        let b = done_rx.recv_timeout(std::time::Duration::from_secs(2));
        let _ = release_tx.send(());
        let _ = a.join();
        assert_eq!(b, Ok(Ok(Some(()))), "clé « b » bloquée par la clé « a »");
    }

    #[test]
    fn other_keys_and_failed_writes_do_not_count() {
        let order = WriteOrder::default();
        let (old, new) = (order.ticket(), order.ticket());
        assert_eq!(order.run("autre", new, || Ok::<_, ()>(1)), Ok(Some(1)));
        assert_eq!(
            order.run("k", new, || Err::<(), _>("refusé")),
            Err("refusé")
        );
        assert_eq!(order.run("k", old, || Ok::<_, ()>(2)), Ok(Some(2)));
    }
}
