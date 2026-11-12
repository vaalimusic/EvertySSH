use parking_lot::RwLock;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate {
        current_version: String,
    },
    Available {
        current_version: String,
        latest_version: String,
        release_url: String,
        release_name: String,
    },
    Error(String),
}

pub fn parse_version(v: &str) -> Vec<u64> {
    let clean = v.trim().trim_start_matches('v').trim_start_matches('V');
    clean
        .split('.')
        .filter_map(|part| {
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<u64>().ok()
        })
        .collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let l_parts = parse_version(latest);
    let c_parts = parse_version(current);
    l_parts > c_parts
}

pub fn check_github_release(current_ver: &str) -> UpdateStatus {
    let url = "https://api.github.com/repos/vaalimusic/EvertySSH/releases/latest";
    let agent = format!("EvertySSH/{}", current_ver);

    let resp = match ureq::get(url)
        .set("User-Agent", &agent)
        .set("Accept", "application/vnd.github.v3+json")
        .timeout(std::time::Duration::from_secs(6))
        .call()
    {
        Ok(r) => r,
        Err(e) => return UpdateStatus::Error(format!("{}", e)),
    };

    let json: serde_json::Value = match resp.into_json() {
        Ok(j) => j,
        Err(e) => return UpdateStatus::Error(format!("Ошибка разбора ответа: {}", e)),
    };

    let tag_name = json.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
    if tag_name.is_empty() {
        return UpdateStatus::Error("Не найден тег релиза".into());
    }

    let release_url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://github.com/vaalimusic/EvertySSH/releases/latest")
        .to_string();

    let release_name = json
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(tag_name)
        .to_string();

    if is_newer(tag_name, current_ver) {
        UpdateStatus::Available {
            current_version: current_ver.to_string(),
            latest_version: tag_name.to_string(),
            release_url,
            release_name,
        }
    } else {
        UpdateStatus::UpToDate {
            current_version: current_ver.to_string(),
        }
    }
}

pub type SharedUpdateStatus = Arc<RwLock<UpdateStatus>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_and_compare_versions() {
        assert_eq!(parse_version("v0.2.5"), vec![0, 2, 5]);
        assert_eq!(parse_version("0.2.6"), vec![0, 2, 6]);
        assert_eq!(parse_version("V1.0.0-rc1"), vec![1, 0, 0]);

        assert!(is_newer("v0.2.6", "0.2.5"));
        assert!(is_newer("v0.3.0", "v0.2.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("v0.2.5", "v0.2.5"));
        assert!(!is_newer("v0.2.4", "v0.2.5"));
    }
}
