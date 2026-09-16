use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub selection_bg: String,
    pub selection_fg: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            selection_bg: "blue".into(),
            selection_fg: "white".into(),
        }
    }
}
