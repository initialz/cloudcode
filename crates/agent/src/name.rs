use gethostname::gethostname;

/// `<hostname>-<user>`, normalised: lower-case ASCII alphanumeric + `-`/`_`,
/// other characters replaced with `-`, truncated to 63 chars.
pub fn default_agent_name() -> String {
    let host_os = gethostname();
    let host = host_os.to_string_lossy();
    let host = host.split('.').next().unwrap_or("unknown");
    let user = whoami::username();
    sanitize(&format!("{}-{}", host, user))
}

fn sanitize(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if out.is_empty() {
        out.push_str("agent");
    }
    if out.len() > 63 {
        out.truncate(63);
    }
    out
}
