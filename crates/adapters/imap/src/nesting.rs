//! Préfiltre des réponses avant le décodeur `imap-codec` : profondeur de
//! parenthèses et repérage des littéraux.
//!
//! `imap-codec` 2.0.0-alpha.9 ne borne pas la récursion des
//! `message/rfc822` imbriqués dans BODYSTRUCTURE (compteur remis à zéro à
//! chaque niveau) : quelques dizaines de Ko suffisent à faire déborder la
//! pile, ce qui arrête le processus. On mesure donc la profondeur de chaque
//! réponse (hors chaînes entre guillemets et hors littéraux) avant de la
//! transmettre ; au-delà de [`MAX_NESTING`], elle n'est pas décodée.

/// Profondeur de parenthèses maximale d'une réponse. Une réponse légitime
/// la plus profonde (BODYSTRUCTURE à 8 niveaux de multipart, limite
/// d'`imap-codec`, avec enveloppes) reste loin en dessous.
pub(crate) const MAX_NESTING: usize = 48;

/// Analyse d'une réponse, ligne par ligne.
#[derive(Debug, Default)]
pub(crate) struct Nesting {
    depth: usize,
    max: usize,
    in_quote: bool,
    escape: bool,
}

impl Nesting {
    /// Analyse une ligne (jusqu'à `\n` compris, hors littéral). Renvoie la
    /// taille du littéral annoncé en fin de ligne (`{n}` ou `{n+}` juste
    /// avant CRLF), qui suit la ligne et ne doit pas être analysé.
    pub(crate) fn scan_line(&mut self, line: &[u8]) -> Option<u64> {
        for &b in line {
            if self.in_quote {
                if self.escape {
                    self.escape = false;
                } else if b == b'\\' {
                    self.escape = true;
                } else if b == b'"' || b == b'\n' {
                    self.in_quote = false;
                }
                continue;
            }
            match b {
                b'"' => self.in_quote = true,
                b'(' => {
                    self.depth += 1;
                    self.max = self.max.max(self.depth);
                }
                b')' => self.depth = self.depth.saturating_sub(1),
                _ => {}
            }
        }
        literal_size(line)
    }

    /// Profondeur maximale atteinte dans la réponse.
    pub(crate) fn max_depth(&self) -> usize {
        self.max
    }
}

/// Taille d'un littéral annoncé en fin de ligne : `…{123}\r\n` ou
/// `…{123+}\r\n` (et `~{123}` pour un littéral binaire).
fn literal_size(line: &[u8]) -> Option<u64> {
    let body = line.strip_suffix(b"\r\n")?.strip_suffix(b"}")?;
    let body = body.strip_suffix(b"+").unwrap_or(body);
    let open = body.iter().rposition(|&b| b == b'{')?;
    let digits = body.get(open + 1..)?;
    if digits.is_empty() || digits.len() > 19 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_detection() {
        assert_eq!(literal_size(b"* 1 FETCH (BODY[] {12}\r\n"), Some(12));
        assert_eq!(literal_size(b"A1 APPEND x {5+}\r\n"), Some(5));
        assert_eq!(literal_size(b"* 1 FETCH (BINARY[1] ~{3}\r\n"), Some(3));
        assert_eq!(literal_size(b"* OK {12}\n"), None);
        assert_eq!(literal_size(b"* OK {}\r\n"), None);
        assert_eq!(literal_size(b"* OK {1a}\r\n"), None);
        assert_eq!(literal_size(b"* OK texte}\r\n"), None);
        assert_eq!(literal_size(b"* OK {99999999999999999999}\r\n"), None);
    }

    #[test]
    fn depth_ignores_quotes() {
        let mut n = Nesting::default();
        assert_eq!(
            n.scan_line(b"* 1 FETCH (FLAGS (\\Seen) X \"((((\\\"((\")\r\n"),
            None
        );
        assert_eq!(n.max_depth(), 2);
    }

    #[test]
    fn deep_rfc822_nesting_is_measured() {
        let mut line = b"* 1 FETCH (BODYSTRUCTURE ".to_vec();
        for _ in 0..500 {
            line.extend_from_slice(b"(\"message\" \"rfc822\" NIL NIL NIL \"7bit\" 1 (NIL NIL NIL NIL NIL NIL NIL NIL NIL NIL) ");
        }
        line.extend_from_slice(b"\r\n");
        let mut n = Nesting::default();
        n.scan_line(&line);
        assert!(n.max_depth() > MAX_NESTING);
    }

    #[test]
    fn unbalanced_closing_does_not_underflow() {
        let mut n = Nesting::default();
        n.scan_line(b"* OK ))))) (\r\n");
        assert_eq!(n.max_depth(), 1);
    }
}
