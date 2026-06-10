//! Prefixed colored log multiplexing metadata for workflow dev sessions.

use std::collections::HashMap;

use colored::Colorize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServiceColor {
    Cyan,
    Magenta,
    Yellow,
    Green,
    Blue,
}

#[derive(Debug, Clone)]
pub struct ServiceLogSource {
    pub name: String,
    pub color: ServiceColor,
}

impl ServiceLogSource {
    pub fn new(name: impl Into<String>, color: ServiceColor) -> Self {
        Self {
            name: name.into(),
            color,
        }
    }

    pub fn format_prefix(&self, use_color: bool) -> String {
        let label = format!("[{}]", self.name);
        if use_color {
            match self.color {
                ServiceColor::Cyan => label.cyan().bold().to_string(),
                ServiceColor::Magenta => label.magenta().bold().to_string(),
                ServiceColor::Yellow => label.yellow().bold().to_string(),
                ServiceColor::Green => label.green().bold().to_string(),
                ServiceColor::Blue => label.blue().bold().to_string(),
            }
        } else {
            label
        }
    }

    pub fn format_line(&self, message: &str, use_color: bool) -> String {
        format!("{} {}", self.format_prefix(use_color), message)
    }
}

#[derive(Debug, Clone)]
pub struct ServiceLogRegistry {
    sources: HashMap<String, ServiceLogSource>,
    palette: [ServiceColor; 5],
    next_color: usize,
}

impl Default for ServiceLogRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceLogRegistry {
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
            palette: [
                ServiceColor::Cyan,
                ServiceColor::Magenta,
                ServiceColor::Yellow,
                ServiceColor::Green,
                ServiceColor::Blue,
            ],
            next_color: 0,
        }
    }

    pub fn register(&mut self, name: impl Into<String>) -> &ServiceLogSource {
        let key = name.into();
        if !self.sources.contains_key(&key) {
            let color = self.palette[self.next_color % self.palette.len()];
            self.next_color += 1;
            self.sources.insert(key.clone(), ServiceLogSource::new(key.clone(), color));
        }
        self.sources.get(&key).expect("source just inserted")
    }

    pub fn get(&self, name: &str) -> Option<&ServiceLogSource> {
        self.sources.get(name)
    }

    pub fn sources(&self) -> impl Iterator<Item = &ServiceLogSource> {
        self.sources.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_assigns_unique_colors() {
        let mut registry = ServiceLogRegistry::new();
        registry.register("frontend");
        registry.register("agent");
        let a = registry.get("frontend").expect("frontend");
        let b = registry.get("agent").expect("agent");
        assert_ne!(a.color, b.color);
        assert!(a.format_prefix(false).contains("[frontend]"));
    }
}
