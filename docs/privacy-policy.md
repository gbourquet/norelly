# Privacy policy

*Version française : [politique-de-confidentialite.md](politique-de-confidentialite.md).*

Norelly is an email client that runs locally on your device. This policy
describes the data Norelly processes and how it is processed. It covers
the Norelly application and its website, norelly.fr.

## Data stored on your device only

- Your messages, attachments and address books are stored locally
  (database and files on your machine).
- Your passwords and session tokens (OAuth) are kept in your operating
  system's secure keyring. If your system has no keyring, Norelly offers
  to keep them in a file on your device, readable by you only and not
  encrypted; it only does so with your consent.
- No data is sent to the Norelly developers: no telemetry, no usage
  statistics, no advertising.

## Connections to your email providers

Norelly connects directly to your email providers' IMAP and SMTP servers,
and to their OAuth sign-in services (Microsoft, Google) when you ask it
to. These connections are encrypted (TLS) and only take place between
your device and those servers.

## Other connections

Norelly connects to nothing else, except in the following cases, all
linked to an action or a choice of yours:

- **Adding an account**: to find your email settings, Norelly asks your
  provider's website, then, if it does not answer, Mozilla's configuration
  database (autoconfig.thunderbird.net), which only receives the domain
  name of your address (for example "example.com"), never the full
  address. It also looks up the DNS records of that domain and tries your
  provider's usual server names (for example "imap.example.com").
- **Address book**: if you link it to a contacts server, Norelly connects
  to it, with the address and password you entered.
- **Images in a message**: images hosted elsewhere are blocked by default.
  If you allow them for a sender, they are loaded from the site that hosts
  them, without cookies.
- **Updates** (Windows installer): if you agree, Norelly checks with
  GitHub, where it is published, whether a new version exists.

## Google and Microsoft accounts

When you choose "Sign in with Google" or "Sign in with Microsoft", Norelly
asks you, on Google's or Microsoft's page, only for the access it needs to
do what an email client does:

- **Google**: access to your Gmail mailbox, to receive, display, organise,
  delete and send your messages, and your email address, to check that the
  account you chose is the one you are adding.
- **Microsoft**: receiving (IMAP) and sending (SMTP) the messages of your
  Outlook.com (or Hotmail, Live) mailbox, staying signed in without asking
  for your password again, and your email address and name, to check that
  the account you chose is the one you are adding.

Norelly does not ask for access to your contacts, your calendar or your
files at Google or Microsoft.

This data is only used to show, organise, delete and send your messages
in Norelly. It stays on your device:

- it is never sold, never transferred to third parties, and never sent to
  the Norelly developers;
- it is never used for advertising, nor to assess your creditworthiness;
- nobody reads it in your place: no person has access to your messages,
  unless you choose to send them yourself (for example in a bug report);
- it is not used to train or improve an artificial intelligence model.

Norelly's use and transfer to any other app of information received from
Google APIs will adhere to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

You can withdraw this access at any time: remove the account in Norelly,
which then erases the authorisation from your device, or remove Norelly's
access in the security settings of your Google or Microsoft account.

## The norelly.fr website

The norelly.fr website sets no cookies and contains no trackers: no visit
statistics, no advertising, no content loaded from another site.

## Deleting data

Uninstalling Norelly, or removing an account in the application, deletes
the associated local data from your device.

## Publisher and contact

Norelly is free software developed by Guillaume Bourquet —
contact: contact@norelly.fr.
Source code: https://github.com/gbourquet/norelly

*Last updated: 22 November 2026.*
