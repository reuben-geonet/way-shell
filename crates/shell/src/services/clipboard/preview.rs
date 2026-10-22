use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use ureq::ResponseExt;
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Preview {
    pub title: String,
    pub description: String,
    #[serde(skip)]
    pub image: Option<Vec<u8>>,
}
pub fn link(url: &str, cancel: Arc<AtomicBool>) -> Result<Preview, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    fn load(
        agent: &ureq::Agent,
        url: &str,
        cancel: &AtomicBool,
    ) -> Result<(String, String, Vec<u8>), String> {
        if cancel.load(Ordering::Acquire) {
            return Err("Preview cancelled".into());
        }
        let mut response = agent
            .get(url)
            .header("User-Agent", "WayShell-Clipboard/1.0")
            .call()
            .map_err(|e| e.to_string())?;
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let final_url = response.get_uri().to_string();
        let mut bytes = Vec::new();
        let mut reader = response.body_mut().as_reader();
        let mut buffer = [0; 16 * 1024];
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Preview cancelled".into());
            }
            let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        Ok((content_type, final_url, bytes))
    }
    let (content_type, final_url, bytes) = load(&agent, url, &cancel)?;
    if content_type.starts_with("image/") {
        return Ok(Preview {
            image: Some(bytes),
            ..Default::default()
        });
    }
    let html = scraper::Html::parse_document(&String::from_utf8_lossy(&bytes));
    let metas = scraper::Selector::parse("meta").unwrap();
    let mut values = std::collections::BTreeMap::new();
    for element in html.select(&metas) {
        if let (Some(key), Some(value)) = (
            element
                .value()
                .attr("property")
                .or_else(|| element.value().attr("name")),
            element.value().attr("content"),
        ) {
            values.insert(key.to_owned(), value.to_owned());
        }
    }
    let title = ["og:title", "twitter:title", "title"]
        .iter()
        .find_map(|key| values.get(*key))
        .cloned()
        .unwrap_or_else(|| {
            html.select(&scraper::Selector::parse("title").unwrap())
                .next()
                .map(|element| element.text().collect())
                .unwrap_or_default()
        });
    let description = ["og:description", "twitter:description", "description"]
        .iter()
        .find_map(|key| values.get(*key))
        .cloned()
        .unwrap_or_default();
    let image = ["og:image", "og:image:url", "twitter:image", "image"]
        .iter()
        .find_map(|key| values.get(*key))
        .and_then(|value| url::Url::parse(&final_url).ok()?.join(value).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .and_then(|url| load(&agent, url.as_str(), &cancel).ok())
        .filter(|(kind, _, _)| kind.starts_with("image/"))
        .map(|(_, _, bytes)| bytes);
    Ok(Preview {
        title,
        description,
        image,
    })
}
