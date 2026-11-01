//! Test de fumée : les serveurs de test démarrent et répondent avec une
//! bannière conforme au protocole.

use std::time::Duration;

use norelly_test_support::containers::{
    DOVECOT_IMAP_PORT, MAILPIT_SMTP_PORT, read_banner, start_dovecot, start_mailpit,
};

const BANNER_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn dovecot_sends_imap_greeting_with_starttls() -> Result<(), Box<dyn std::error::Error>> {
    let server = start_dovecot().await?;
    let port = server.port(DOVECOT_IMAP_PORT).await?;
    let banner = read_banner(&server.host, port, BANNER_TIMEOUT).await?;

    assert!(
        banner.starts_with("* OK"),
        "bannière IMAP inattendue : {banner}"
    );
    assert!(
        banner.contains("STARTTLS"),
        "STARTTLS non annoncé : {banner}"
    );
    Ok(())
}

#[tokio::test]
async fn mailpit_sends_smtp_greeting() -> Result<(), Box<dyn std::error::Error>> {
    let server = start_mailpit().await?;
    let port = server.port(MAILPIT_SMTP_PORT).await?;
    let banner = read_banner(&server.host, port, BANNER_TIMEOUT).await?;

    assert!(
        banner.starts_with("220 "),
        "bannière SMTP inattendue : {banner}"
    );
    Ok(())
}
