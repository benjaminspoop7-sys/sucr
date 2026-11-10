//! The update check: one small release file, read through the engine's own TLS
//! trust and proxy, compared with the version this build was compiled with.
//!
//! Two shapes are read, so the file can be either hand-written or the release
//! JSON GitHub and GitLab answer with:
//!
//! ```json
//! { "version": "0.2.0", "url": "https://host/Sucr_0.2.0_x64-setup.exe" }
//! ```
//!
//! or `{ "tag_name": "v0.2.0", "assets": [{ "browser_download_url": "…" }] }`.
//!
//! Nothing is downloaded here and nothing is installed: the app says a newer
//! build exists and hands the link to the browser. An installer that replaces
//! itself is a different decision.

use serde_json::{json, Value};
use std::time::Duration;

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// The feed a normal install checks, so the user never types an address. The
/// file is compiled in, which makes moving hosts a one-line edit; a fork can
/// point its build somewhere else with `SUCR_UPDATE_FEED` without touching it.
pub const DEFAULT_FEED: &str = match option_env!("SUCR_UPDATE_FEED") {
    Some(url) => url,
    None => include_str!("../update-feed.txt"),
};

fn default_feed() -> Option<&'static str> {
    let feed = DEFAULT_FEED.trim();
    (!feed.is_empty()).then_some(feed)
}

/// `1.2.3` → (1, 2, 3). Release tags carry a `v` and may carry a suffix; neither
/// changes which build is newer, so both are dropped before the compare.
fn parts(raw: &str) -> Option<(u64, u64, u64)> {
    let head = raw.trim().trim_start_matches(['v', 'V']);
    let head = head.split(['-', '+']).next()?;
    let mut it = head.split('.');
    let a = it.next()?.trim().parse().ok()?;
    let b = it.next().unwrap_or("0").trim().parse().ok()?;
    let c = it.next().unwrap_or("0").trim().parse().ok()?;
    Some((a, b, c))
}

fn newer(latest: &str, current: &str) -> bool {
    match (parts(latest), parts(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// The release JSON as (version, link, notes), in either shape.
fn read_release(body: &str) -> Option<(String, Option<String>, Option<String>)> {
    let doc: Value = serde_json::from_str(body).ok()?;
    let text = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let version = text(&doc, "version").or_else(|| text(&doc, "tag_name"))?;
    let url = text(&doc, "url")
        .or_else(|| text(&doc, "download"))
        .or_else(|| {
            // The first asset that is an installer; a release page also carries
            // the add-on and checksum files, and those are not what the button
            // is for. GitHub sends a list of assets, GitLab an object holding a
            // `links` list, so both containers are read.
            let assets = doc.get("assets")?;
            let list = if let Some(arr) = assets.as_array() {
                arr.clone()
            } else {
                assets.get("links")?.as_array()?.clone()
            };
            list.iter().find_map(|a| {
                let link = text(a, "browser_download_url")
                    .or_else(|| text(a, "url"))
                    .or_else(|| text(a, "direct_asset_url"));
                link.filter(|l| l.ends_with("-setup.exe"))
            })
        })
        .or_else(|| text(&doc, "html_url"));
    let notes = text(&doc, "notes").or_else(|| text(&doc, "body"));
    Some((version, url, notes))
}

fn link_ok(url: &str) -> bool {
    (url.starts_with("http://") || url.starts_with("https://")) && !url.contains(char::is_whitespace)
}

fn answer(ok: bool, latest: Option<String>, notes: Option<String>, error: Option<String>) -> Value {
    let available = latest.as_deref().map(|l| newer(l, CURRENT)).unwrap_or(false);
    json!({
        "ok": ok,
        "current": CURRENT,
        "latest": latest,
        "available": available,
        "notes": notes,
        "error": error,
    })
}

/// `feed` is the address the user saved, if any; empty or absent means the
/// built-in one, which is the normal path.
pub fn check(feed: Option<&str>) -> Value {
    let Some(feed) = feed.map(str::trim).filter(|f| !f.is_empty()).or(default_feed()) else {
        return answer(false, None, None, Some("This build has no release feed yet.".into()));
    };
    if !link_ok(feed) {
        return answer(false, None, None, Some("The release feed has to be an http or https address.".into()));
    }
    let agent = crate::engine::config_connect(Duration::from_secs(10)).new_agent();
    let body = match agent.get(feed).call() {
        Ok(mut resp) => {
            let status = resp.status().as_u16();
            if !(200..300).contains(&status) {
                // A release host answers 404 until the first release exists, which
                // is a normal state for a new build rather than a broken feed.
                let why = if status == 404 || status == 410 {
                    "No release has been published for this build yet.".to_string()
                } else {
                    format!("The release feed answered {status}.")
                };
                return answer(false, None, None, Some(why));
            }
            match resp.body_mut().read_to_string() {
                Ok(body) => body,
                Err(e) => return answer(false, None, None, Some(format!("The feed stopped halfway: {e}"))),
            }
        }
        Err(e) => return answer(false, None, None, Some(format!("The feed could not be read: {e}"))),
    };
    match read_release(&body) {
        Some((version, url, notes)) => {
            let available = newer(&version, CURRENT);
            let url = url.filter(|u| link_ok(u));
            let error: Option<String> = if available && url.is_none() {
                Some("The feed names a newer build but carries no download link.".into())
            } else {
                None
            };
            json!({
                "ok": error.is_none(),
                "current": CURRENT,
                "latest": version,
                "available": available && url.is_some(),
                "url": url,
                "notes": notes,
                "error": error,
            })
        }
        None => answer(false, None, None, Some("The feed is not a release file Sucr can read.".into())),
    }
}

/// Hands the link to Windows, which opens it in the default browser. The app
/// never fetches the installer itself: a browser download is the user's, with
/// their own proxy, disk and malware scan.
pub fn open(url: &str) -> Value {
    if !link_ok(url) {
        return json!({ "ok": false, "current": CURRENT, "available": false,
                       "error": "A download link has to be an http or https address." });
    }
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let code = unsafe {
            ShellExecuteW(
                None,
                &HSTRING::from("open"),
                &HSTRING::from(url),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        // ShellExecute reports success as anything above 32, never as zero.
        if code.0 as isize > 32 {
            json!({ "ok": true, "current": CURRENT, "available": false })
        } else {
            json!({ "ok": false, "current": CURRENT, "available": false,
                    "error": "Windows could not open that link." })
        }
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        json!({ "ok": false, "current": CURRENT, "available": false,
                "error": "Only the Windows build opens a link." })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_with_a_v_and_a_suffix_still_compares() {
        assert!(newer("v0.2.0", "0.1.0"));
        assert!(newer("0.1.10", "0.1.9"));
        assert!(newer("1.0.0-beta.1", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("v0.0.9", "0.1.0"));
        assert!(!newer("latest", "0.1.0"));
    }

    #[test]
    fn both_release_shapes_read() {
        let own = read_release(
            r#"{"version":"0.4.0","url":"https://h/x/Sucr_0.4.0_x64-setup.exe","notes":"a"}"#,
        )
        .unwrap();
        assert_eq!(own.0, "0.4.0");
        assert_eq!(own.1.as_deref(), Some("https://h/x/Sucr_0.4.0_x64-setup.exe"));

        let gh = read_release(
            r#"{"tag_name":"v0.5.0","body":"b","assets":[
                 {"name":"addon.zip","browser_download_url":"https://h/x/theme.zip"},
                 {"name":"Sucr_0.5.0_x64-setup.exe","browser_download_url":"https://h/x/Sucr_0.5.0_x64-setup.exe"}]}"#,
        )
        .unwrap();
        assert_eq!(gh.0, "v0.5.0");
        assert_eq!(gh.1.as_deref(), Some("https://h/x/Sucr_0.5.0_x64-setup.exe"));

        let gl = read_release(
            r#"{"tag_name":"v0.6.0","assets":{"links":[{"name":"setup","url":"https://g/-/x/Sucr_0.6.0_x64-setup.exe"}]}}"#,
        )
        .unwrap();
        assert_eq!(gl.1.as_deref(), Some("https://g/-/x/Sucr_0.6.0_x64-setup.exe"));

        assert!(read_release("{}").is_none());
        assert!(read_release("<html>").is_none());
    }

    /// The built-in feed is what the button uses with no saved setting, so it has
    /// to be absent (a build with no host yet) or a real http address — never
    /// something the request would refuse later.
    #[test]
    fn the_built_in_feed_is_absent_or_an_http_address() {
        match default_feed() {
            None => {}
            Some(url) => assert!(link_ok(url), "built-in feed is not a web address: {url}"),
        }
    }

    #[test]
    fn a_feed_that_is_not_a_web_address_is_refused_before_any_request() {
        // No `check(None)` here: that reads the built-in feed, and this test must
        // not reach the network. An unusable address covers the refusal path.
        let v = check(Some("file:///C:/feed.json"));
        assert_eq!(v["ok"], false);
        let v = check(Some("feed.json"));
        assert_eq!(v["ok"], false);
        assert_eq!(v["current"], CURRENT);
        let v = open("javascript:alert(1)");
        assert_eq!(v["ok"], false);
    }
}
