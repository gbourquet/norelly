// Étiquette affichée d'une adresse ou d'un téléphone de contact : traduite
// quand Rust en a reconnu le genre (travail, domicile, mobile…), texte
// enregistré sinon.

import type { ContactLabelKindDto } from "./ipc/api";
import { m } from "./paraglide/messages.js";

/** Traduction d'un genre d'étiquette, lue à chaque appel (langue
 * courante). */
export function labelKindText(kind: ContactLabelKindDto): string {
  switch (kind) {
    case "work":
      return m.contact_label_kind_work();
    case "home":
      return m.contact_label_kind_home();
    case "mobile":
      return m.contact_label_kind_mobile();
    case "fax":
      return m.contact_label_kind_fax();
    case "pager":
      return m.contact_label_kind_pager();
    case "main":
      return m.contact_label_kind_main();
    case "other":
      return m.contact_label_kind_other();
  }
}

/** Étiquette d'une adresse ou d'un téléphone dans l'interface. */
export function contactLabel(row: {
  label: string;
  label_kind?: ContactLabelKindDto | null;
}): string {
  return row.label_kind == null ? row.label : labelKindText(row.label_kind);
}
