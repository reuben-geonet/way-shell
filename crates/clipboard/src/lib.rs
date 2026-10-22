//! Clipboard history policy and storage, independent of GTK and Wayland.
pub mod actions;
pub mod content;
pub mod store;

pub use content::{Content, HighlightSpan, Kind, classify, highlight};
pub use store::{Entry, HistoryPolicy, Retention, Store};

pub const TAGS: [&str; 9] = [
    "blue", "teal", "green", "yellow", "orange", "red", "pink", "purple", "slate",
];

#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub query: String,
    pub kind: Option<Kind>,
    pub pinned: bool,
    pub tag: Option<String>,
    pub exclude_pinned: bool,
    pub exclude_tagged: bool,
}
impl Filter {
    pub fn matches(&self, entry: &Entry) -> bool {
        if self.kind.is_some_and(|kind| kind != entry.content.kind)
            || (self.pinned && !entry.pinned)
            || (!self.pinned && self.exclude_pinned && entry.pinned)
            || self
                .tag
                .as_ref()
                .is_some_and(|tag| entry.tag.as_ref() != Some(tag))
            || (self.tag.is_none() && self.exclude_tagged && entry.tag.is_some())
        {
            return false;
        }
        let query = search_key(&self.query);
        query.is_empty()
            || [
                &entry.title,
                &entry.content.text,
                &entry.content.metadata.to_string(),
            ]
            .iter()
            .any(|value| search_key(value).contains(&query))
    }
}
fn search_key(text: &str) -> String {
    use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
    text.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}
