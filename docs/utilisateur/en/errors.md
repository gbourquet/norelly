# What to do when an error message appears

[← Back to contents](README.md)

Norelly's error messages say what is happening and, most of the time, what
to do. Here are the most common ones, explained in more detail.

## Unrecognised certificate

**What you see**: an **Unrecognised certificate** window, with a server
name, a reason (for example "The certificate has expired.") and a long
string of digits and letters called the **fingerprint**.

![The "Unrecognised certificate" warning](captures/certificate.png)

**What it means**: Norelly could not check the server's identity. Either
the server is badly configured, or someone is pretending to be it (on
public Wi-Fi, for example) to read your messages.

**What to do**:

- With a large provider (Gmail, Outlook.com, Fastmail…), **click Cancel**.
  Try again later, at home rather than on public Wi-Fi. If the problem
  persists, check that your computer's date and time are correct: a wrong
  clock makes certificates "not valid yet" or "expired".
- With a personal server or your company's, ask its administrator for the
  fingerprint of its certificate. If it is identical to the one shown,
  click **Trust this server**.

**If the message says "this server presents a certificate different from
the one you had accepted"**: be careful. The server has changed
certificate, which happens on renewal, but also during an interception.
Only continue after checking the new fingerprint with the administrator.

## Offline

**What you see**: a small crossed-out Wi-Fi icon next to the account's
address, in the left panel. Hovering over it shows the reason (for example
"Offline: server unreachable").

**What it means**: Norelly cannot reach your provider: your computer is not
connected to the Internet, or the server is not answering at the moment.

**What to do**: nothing in particular. Norelly retries by itself, at
growing intervals, and resumes as soon as the connection is back.
Meanwhile:

- you can **read and search** every message already downloaded;
- you can **archive, delete, move**: these actions are kept and sent to the
  server when the connection returns;
- you can **write and send**: the message waits in the outbox and goes out
  by itself later.

If the icon stays while the Internet works (you can open a website in your
browser), ask for a synchronisation: click the **⋯** button next to the
account's address, then **Synchronise**. If that is not enough, your provider may have a temporary
problem: try again later.

## Credentials rejected (password refused)

**What you see**: "Incoming server (IMAP): credentials rejected. Check the
username and password (an app password is sometimes required)."

**What it means**: your provider refuses the address or the password.

**What to do**:

1. Check the password: is **Caps Lock** on? Is the keyboard in the right
   language?
2. **Gmail, Outlook.com**: use an **app password**, not your usual password
   (see [Adding your address](first-account.md#gmail-and-outlookcom-the-app-password)).
   If you changed your Google or Microsoft password, older app passwords may
   have been revoked: create a new one.
3. If Norelly shows the server settings, check the **Username** field: it is
   often the full address (`mary.smith@example.com`), not just `mary.smith`.
4. Try signing in on your provider's website: your account may be
   temporarily locked after several attempts.

## Server unreachable

**What you see**: "server unreachable. Check the name and port."

**What to do**: check your Internet connection. If Norelly shows the server
settings, compare **Server** and **Port** with your provider's help page: a
single wrong letter is enough to prevent the connection.

## Could not send

**What you see**: at the bottom of the window, "Could not send "message
subject" (reason)".

**What to do**: open the **outbox** (**⋯** button next to the account's
address, then **Outbox**). The
message is there:

- **Retry** sends it again;
- **Cancel sending** removes it from the queue: it will not be sent.

If the reason is "interrupted, may have been sent", Norelly was closed or
the connection cut at the very moment of sending. The message may have
arrived: look in the **Sent** folder on your provider's website, or ask the
recipient, before clicking **Retry**, so as not to send it twice.

## Other messages

| Message | What to do |
|---|---|
| "No keyring" | Your Linux has no active password safe. Install or start GNOME Keyring or KWallet (ask for help if needed). Otherwise, Norelly offers to store the password in an **unencrypted** file: only accept if nobody else uses your session. |
| "Unencrypted connection" | The server offers no protected connection. Only click **Allow** for a server you know, on your own network; otherwise click **Edit settings** and choose **TLS**. |
| "This account has no archive folder" | Choose once and for all the folder to archive into: **⋯** button next to the account's address, then **Choose archive folder…**. |
| "Attachments exceed 25 MiB." | Remove some files, or send them in several messages. For very large files, use a file-sharing service. |
| "Too late to undo" | The action has already been done on the server. Do the opposite action (for example, move the message from the Trash back to the Inbox). |
| "Executable file type" | Norelly refuses to open a program received by email directly. Use **Save…** if you really were expecting it. |
| "Remote images blocked" | This is not an error: see [blocked images](everyday.md#blocked-images). |
