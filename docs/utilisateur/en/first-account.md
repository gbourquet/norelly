# Adding your email address

[← Back to contents](README.md)

In Norelly, each email address is called an **account**. You can add
several (for example a personal address and one for a club).

## What you need

- your **email address**, for example `mary.smith@example.com`;
- its **password**. For Gmail and Outlook.com, this is not your usual
  password but an **app password**: first read
  [the section about them](#gmail-and-outlookcom-the-app-password).

## Step by step

1. Click **Add an account**: on the welcome screen if it is your first
   account, otherwise on the **+** button at the top of the left panel.
2. Type your **email address** and your **password**. The **display name**
   is optional: it is the name your contacts will see next to your address
   (for example "Mary Smith").

   ![The "Add an account" window, with the address, password and display name filled in](captures/add-account.png)

3. Click **Continue**. Norelly looks up your provider's settings by itself:
   wait a few seconds.
4. Most of the time, that's all: Norelly tests the connection, then your
   address appears in the left panel and your messages arrive. The first
   time, downloading your messages can take a few minutes if you have many.
   You can already read the first ones.

Your password is stored in your system's **keyring** (the password safe of
Windows or Linux). The only exception: on Linux, if no keyring responds,
Norelly may offer to store it in an **unencrypted** file; it only does so
if you accept (see [error messages](errors.md#other-messages)).

### If Norelly shows you the server settings

When Norelly cannot find the settings by itself, it shows two boxes:
**Incoming (IMAP)** and **Outgoing (SMTP)**. Your provider gives this
information on its help pages (search for "IMAP" and "SMTP" with your
provider's name).

![The server settings: server, port, security and username, for incoming and outgoing mail](captures/servers.png)

- **Server**: a name such as `imap.example.com` (incoming) or
  `smtp.example.com` (outgoing);
- **Port**: a number, most often `993` for incoming and `465` or `587` for
  outgoing;
- **Security**: keep **TLS (recommended)**, or **STARTTLS** if your
  provider says so (often with port `587` or `143`). Only choose **None
  (unencrypted)** if you know exactly why;
- **Username**: most often your full address.

Then click **Test and add**.

### If Norelly talks about a certificate

A **certificate** is a server's identity card. If Norelly cannot check it,
it stops and shows this screen.

![The "Unrecognised certificate" warning, with the server name, the reason and the fingerprint](captures/certificate.png)

With large providers (Gmail, Outlook.com, Fastmail…) this screen **should
never appear**: if it does, click **Cancel** and try again later, from
another network (your home connection rather than public Wi-Fi). See also
[error messages](errors.md#unrecognised-certificate).

Only click **Trust this server** for a server you know (a personal server,
your company's), and if its administrator has confirmed the
**fingerprint** shown.

## Gmail and Outlook.com: the app password

Gmail and Outlook.com refuse your usual password in a program like
Norelly. They ask you to create an **app password**: a special password of
16 letters, reserved for Norelly. You create it once, on your provider's
website, then paste it into Norelly instead of your password.

The benefit: if one day you change your usual password, or want to cut off
Norelly's access, you just delete this app password.

### Gmail

1. First turn on **2-Step Verification** for your Google account, if not
   already done: in your browser, open
   <https://myaccount.google.com/security>, then **2-Step Verification**,
   and follow the instructions (Google will text you a code, for example).
2. Then open <https://myaccount.google.com/apppasswords>. Sign in if Google
   asks you to.
3. In **App name**, type `Norelly`, then click **Create**.
4. Google shows a 16-letter password in a yellow box. Select it, copy it
   (**Ctrl** + **C**), then paste it (**Ctrl** + **V**) into Norelly's
   **Password** field. Spaces do not matter.
5. Click **Done** at Google: this password will never be shown again. That
   is fine: if you lose it, create another one.

### Outlook.com (and Hotmail, Live)

1. Turn on **two-step verification**: in your browser, open
   <https://account.microsoft.com/security>, then **Advanced security
   options**, and turn on **Two-step verification**.
2. On the same page, under **App passwords**, click **Create a new app
   password**.
3. Copy the password shown and paste it into Norelly's **Password** field.

> **Good to know**: Microsoft increasingly restricts password access by
> email programs, in favour of signing in through its own page (which
> Norelly cannot do yet). If Norelly says "credentials rejected" despite a
> correct app password, Microsoft refuses this method for your account: see
> [frequently asked questions](faq.md#outlookcom-refuses-my-app-password).

## Removing an account

In the left panel, click the **⋯** button next to the account's address,
then **Remove this account…**. Norelly asks you to confirm.

Removing an account erases from your computer its copy of the messages and
its password. **Your messages stay with your provider**: you will find them
on its website, or by adding the account again.

Next: [using Norelly every day](everyday.md).
