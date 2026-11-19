-- Version 27 : photo d'un contact.
--
-- photo : empreinte SHA-256 (64 chiffres hexadécimaux minuscules) de la
-- photo réencodée, rangée dans le magasin des photos
-- (norelly_blobs::FsContactPhotoStore) ; NULL sans photo. Plusieurs
-- contacts peuvent porter la même photo (contenu identique). La colonne
-- n'est écrite que par l'import ; modifier ou fusionner une fiche la
-- garde. Une valeur qui n'est pas une empreinte valide est lue comme une
-- absence de photo.
ALTER TABLE contacts ADD COLUMN photo TEXT;

CREATE INDEX contacts_by_photo ON contacts (photo) WHERE photo IS NOT NULL;
