//! Membres des groupes (listes de diffusion) d'un fichier vCard ou LDIF :
//! une fois toutes les fiches lues, chaque membre désigné par une autre
//! fiche du fichier (identifiant vCard, nom distinctif LDIF) est rendu par
//! le rang de cette fiche, sans recopier ses adresses ; le domaine décide
//! du contact qu'il désigne (`plan_import`).

use std::collections::HashMap;

use norelly_domain::contact_import::{ImportedCard, ImportedList, ImportedMember};
use norelly_domain::contacts::ContactInput;

/// Fiche lue, avant que les membres des groupes soient retrouvés.
#[derive(Debug)]
pub(crate) enum Read {
    Ready(ImportedCard),
    /// Contact, et l'identifiant par lequel un groupe peut le désigner.
    Contact(ContactInput, Option<String>),
    /// Groupe, et ses membres tels que lus.
    List(ImportedList, Vec<String>),
}

/// Comment un format désigne un membre.
pub(crate) struct Designation<K, D, F> {
    /// Forme comparable d'un identifiant (celui d'une fiche ou d'un
    /// membre).
    pub key: K,
    /// Adresse donnée directement par le membre (`mailto:`), essayée en
    /// premier.
    pub direct: D,
    /// Adresse tirée du membre quand aucune fiche du fichier ne porte son
    /// identifiant (attribut `mail` d'un nom distinctif).
    pub fallback: F,
}

/// Fiches lues, membres des groupes retrouvés : adresse directe, sinon
/// rang de la première fiche de contact qui porte l'identifiant, sinon
/// adresse de repli ; sinon (ou adresse vide) compté introuvable.
pub(crate) fn resolve<K, D, F>(read: Vec<Read>, how: &Designation<K, D, F>) -> Vec<ImportedCard>
where
    K: Fn(&str) -> String,
    D: Fn(&str) -> Option<String>,
    F: Fn(&str) -> Option<String>,
{
    let mut ids: HashMap<String, usize> = HashMap::new();
    for (index, card) in read.iter().enumerate() {
        if let Read::Contact(_, Some(id)) = card {
            ids.entry((how.key)(id)).or_insert(index);
        }
    }
    read.into_iter()
        .map(|card| match card {
            Read::Ready(card) => card,
            Read::Contact(input, _) => ImportedCard::Contact(input),
            Read::List(mut list, members) => {
                for member in members {
                    let found = match (how.direct)(&member) {
                        Some(address) => Some(ImportedMember::Address(address)),
                        None => ids
                            .get(&(how.key)(&member))
                            .map(|&i| ImportedMember::Card(i))
                            .or_else(|| (how.fallback)(&member).map(ImportedMember::Address)),
                    };
                    match found {
                        Some(ImportedMember::Address(a)) if a.trim().is_empty() => {
                            list.unresolved += 1;
                        }
                        Some(member) => list.members.push(member),
                        None => list.unresolved += 1,
                    }
                }
                ImportedCard::List(list)
            }
        })
        .collect()
}

/// Octet d'un échappement hexadécimal `HH` (deux chiffres hexadécimaux
/// ASCII exactement, sans signe), ou `None`.
pub(crate) fn hex_byte(digits: Option<&str>) -> Option<u8> {
    let digits = digits?;
    if digits.len() != 2 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u8::from_str_radix(digits, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_escapes_need_two_hex_digits() {
        assert_eq!(hex_byte(Some("27")), Some(0x27));
        assert_eq!(hex_byte(Some("aF")), Some(0xAF));
        for bad in ["+b", "-1", "2", "zz", " 2"] {
            assert_eq!(hex_byte(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(hex_byte(None), None);
    }
}
