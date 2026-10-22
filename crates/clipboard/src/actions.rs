use crate::{Entry, Kind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionConfig {
    pub actions: Vec<Action>,
    #[serde(default)]
    pub defaults: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub types: Vec<Kind>,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub space: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub shortcut: Vec<String>,
    #[serde(default)]
    pub actions: Vec<Action>,
}
impl Action {
    pub fn is_group(&self) -> bool {
        self.kind == "submenu" || !self.actions.is_empty()
    }
    pub fn captures(&self, entry: &Entry) -> Option<Vec<String>> {
        if !self.types.is_empty() && !self.types.contains(&entry.content.kind) {
            return None;
        }
        match self.pattern.as_deref().filter(|s| !s.is_empty()) {
            None => Some(Vec::new()),
            Some(pattern) => {
                let regex = regress::Regex::new(pattern).ok()?;
                let result = regex.find(&entry.content.text)?;
                Some(
                    result
                        .captures
                        .iter()
                        .map(|range| {
                            range
                                .as_ref()
                                .map(|r| entry.content.text[r.clone()].to_owned())
                                .unwrap_or_default()
                        })
                        .collect(),
                )
            }
        }
    }
}
impl ActionConfig {
    pub fn find(&self, id: &str) -> Option<&Action> {
        fn find<'a>(actions: &'a [Action], id: &str) -> Option<&'a Action> {
            actions.iter().find_map(|action| {
                if action.id == id {
                    Some(action)
                } else {
                    find(&action.actions, id)
                }
            })
        }
        find(&self.actions, id)
    }
    pub fn validate(&self) -> Result<(), String> {
        fn check(
            actions: &[Action],
            ids: &mut std::collections::BTreeSet<String>,
        ) -> Result<(), String> {
            for action in actions {
                if action.is_group() {
                    check(&action.actions, ids)?;
                    continue;
                }
                if action.id.is_empty() || !ids.insert(action.id.clone()) {
                    return Err("Action IDs must be nonempty and unique".into());
                }
                if !["command", "color", "qrcode", "open", "files", "path"]
                    .contains(&action.kind.as_str())
                {
                    return Err(format!("Unknown action kind: {}", action.kind));
                }
                if let Some(pattern) = &action.pattern {
                    regress::Regex::new(pattern).map_err(|e| e.to_string())?;
                }
                if !["copy", "paste", "ignore"].contains(&action.output.as_str()) {
                    return Err("Action output must be copy, paste or ignore".into());
                }
            }
            Ok(())
        }
        check(&self.actions, &mut Default::default())?;
        for id in self.defaults.values() {
            if self.find(id).is_none_or(Action::is_group) {
                return Err(format!("Unknown default action: {id}"));
            }
        }
        Ok(())
    }
}
impl Default for ActionConfig {
    fn default() -> Self {
        serde_json::from_value(serde_json::json!({"actions":[
            {"kind":"open","id":"open","name":"Open with Default","types":["Image","File","Link"],"output":"ignore"},
            {"kind":"files","id":"files","name":"Open with Files","types":["Image","File","Files"],"output":"ignore"},
            {"kind":"path","id":"path","name":"Paste as Path","types":["Image","File","Files"],"output":"paste"},
            {"name":"Convert","actions":[
                {"kind":"color","id":"rgb","name":"RGB","space":"rgb","types":["Color"],"output":"paste"},
                {"kind":"color","id":"hex","name":"Hex","space":"hex","types":["Color"],"output":"paste"},
                {"kind":"color","id":"hsl","name":"HSL","space":"hsl","types":["Color"],"output":"paste"},
                {"kind":"color","id":"oklch","name":"OKLCH","space":"oklch","types":["Color"],"output":"paste"}]},
            {"kind":"qrcode","id":"qrcode","name":"QR Code","types":["Text","Code","Link","Character","Color"],"output":"ignore","shortcut":["<Control>q"]}
        ],"defaults":{"File":"path","Files":"path","Link":"open"}})).unwrap()
    }
}
pub fn convert_color(text: &str, space: &str) -> Result<String, String> {
    let color = text
        .trim()
        .parse::<csscolorparser::Color>()
        .map_err(|e| e.to_string())?;
    match space {
        "hex" => {
            let [r, g, b, a] = color.to_rgba8();
            Ok(if a == 255 {
                format!("#{r:02x}{g:02x}{b:02x}")
            } else {
                format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
            })
        }
        "rgb" => Ok(color.to_css_rgb().to_string()),
        "hsl" => Ok(color.to_css_hsl().to_string()),
        "hwb" => Ok(color.to_css_hwb().to_string()),
        "lab" => Ok(color.to_css_lab().to_string()),
        "lch" => Ok(color.to_css_lch().to_string()),
        "oklab" => Ok(color.to_css_oklab().to_string()),
        "oklch" => Ok(color.to_css_oklch().to_string()),
        _ => Err("Unknown color space".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regex_capture_arguments_and_defaults_are_validated() {
        let mut config = ActionConfig::default();
        config.validate().unwrap();
        let mut action = config.actions[0].clone();
        action.types = vec![Kind::Text];
        action.pattern = Some(r"(?<=color:)(#[0-9a-f]+)".into());
        let entry = Entry {
            id: 1,
            content: crate::Content::text("color:#3584e4"),
            title: String::new(),
            pinned: false,
            tag: None,
            copied: 0,
        };
        assert_eq!(action.captures(&entry).unwrap(), ["#3584e4"]);
        action.pattern = Some("[".into());
        config.actions.push(action);
        assert!(config.validate().is_err());
        let config: ActionConfig = serde_json::from_value(
            serde_json::json!({"actions":[{"kind":"submenu","name":"Empty submenu"}]}),
        )
        .unwrap();
        config.validate().unwrap();
        assert!(convert_color("#3584e4", "rgb").unwrap().starts_with("rgb("));
    }
}
