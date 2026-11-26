//! `nordlys.toml`, edited in place: your comments and layout stay, and a
//! change is checked against the same parser the shell uses before it's
//! written, so the shell never sees a file it would reject.

use nordlys_config::{Config, Group, OpenIn};
use std::path::PathBuf;
use std::time::SystemTime;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

pub struct NordlysToml {
    pub path: PathBuf,
    doc: DocumentMut,
    pub config: Config,
    /// Why the file on disk couldn't be read, if it couldn't.
    pub problem: Option<String>,
    stamp: Option<SystemTime>,
}

impl NordlysToml {
    pub fn load() -> Self {
        Self::load_from(nordlys_config::default_path())
    }

    pub fn load_from(path: PathBuf) -> Self {
        let stamp = stamp(&path);
        let text = std::fs::read_to_string(&path).ok().or_else(example);
        let (doc, config, problem) = match text {
            None => (DocumentMut::new(), Config::default(), None),
            Some(t) => match (t.parse::<DocumentMut>(), Config::from_toml(&t)) {
                (Ok(d), Ok(c)) => (d, c, None),
                (Ok(d), Err(e)) => (d, Config::default(), Some(e.to_string())),
                (Err(e), _) => (DocumentMut::new(), Config::default(), Some(e.to_string())),
            },
        };
        Self {
            path,
            doc,
            config,
            problem,
            stamp,
        }
    }

    /// Someone else saved the file (an editor, Super+,): read it again.
    pub fn changed_on_disk(&self) -> bool {
        stamp(&self.path) != self.stamp
    }

    /// Apply `f`, check the result and write it. On any error the file and
    /// this copy stay as they were.
    pub fn edit(&mut self, f: impl FnOnce(&mut DocumentMut)) -> Result<(), String> {
        if self.problem.is_some() {
            return Err(format!(
                "{} has a mistake; fix it in Advanced → Edit nordlys.toml first",
                self.path.display()
            ));
        }
        let mut doc = self.doc.clone();
        f(&mut doc);
        let text = doc.to_string();
        let config = Config::from_toml(&text).map_err(|e| e.to_string())?;
        crate::sys::write_atomic(&self.path, &text)?;
        self.doc = doc;
        self.config = config;
        self.stamp = stamp(&self.path);
        Ok(())
    }

    /// Set `[table] key = value`, keeping the comment after it.
    pub fn set(&mut self, table: &str, key: &str, value: impl Into<Value>) -> Result<(), String> {
        let value = value.into();
        self.edit(|doc| set_in(doc, table, key, value))
    }

    /// Drop a key so its default applies.
    pub fn unset(&mut self, table: &str, key: &str) -> Result<(), String> {
        self.edit(|doc| {
            if let Some(t) = doc.get_mut(table).and_then(Item::as_table_mut) {
                t.remove(key);
            }
        })
    }

    pub fn set_groups(&mut self, groups: &[Group]) -> Result<(), String> {
        self.edit(|doc| {
            // Keep the comment above the first group.
            let lead = doc
                .get("group")
                .and_then(Item::as_array_of_tables)
                .and_then(|a| a.get(0))
                .and_then(|t| t.decor().prefix().cloned());
            doc.remove("group");
            if groups.is_empty() {
                return;
            }
            let mut aot = ArrayOfTables::new();
            for (i, g) in groups.iter().enumerate() {
                let mut t = group_table(g);
                if i == 0
                    && let Some(lead) = lead.clone()
                {
                    t.decor_mut().set_prefix(lead);
                }
                aot.push(t);
            }
            doc.insert("group", Item::ArrayOfTables(aot));
        })
    }
}

fn set_in(doc: &mut DocumentMut, table: &str, key: &str, mut value: Value) {
    if !doc.contains_key(table) {
        doc.insert(table, Item::Table(Table::new()));
    }
    let Some(t) = doc.get_mut(table).and_then(Item::as_table_mut) else {
        return;
    };
    if let Some(old) = t.get(key).and_then(Item::as_value) {
        *value.decor_mut() = old.decor().clone();
    }
    t.insert(key, Item::Value(value));
}

fn group_table(g: &Group) -> Table {
    let mut t = Table::new();
    t.insert("name", toml_edit::value(g.name.as_str()));
    if let Some(k) = g.key {
        t.insert("key", toml_edit::value(k.to_string()));
    }
    let mut apps = toml_edit::Array::new();
    for a in &g.apps {
        apps.push(a.as_str());
    }
    t.insert("apps", toml_edit::value(apps));
    t.insert(
        "open_in",
        toml_edit::value(match g.open_in {
            OpenIn::NewWorkspace => "new-workspace",
            OpenIn::Current => "current",
        }),
    );
    if let Some(d) = &g.dir {
        t.insert("dir", toml_edit::value(d.as_str()));
    }
    if !g.reuse_running {
        t.insert("reuse_running", toml_edit::value(false));
    }
    t
}

fn stamp(p: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Start a missing file from the installed example, comments and all.
fn example() -> Option<String> {
    let dir = crate::sys::data_dir()?;
    std::fs::read_to_string(dir.join("nordlys.toml.example")).ok()
}

/// Round to two places so 0.6 stays "0.6" and not 0.6000000238.
pub fn tidy(v: f32) -> f64 {
    (v as f64 * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, contents: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nordlys-settings-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("nordlys.toml");
        std::fs::write(&p, contents).unwrap();
        p
    }

    #[test]
    fn edits_keep_comments() {
        let p = temp(
            "keep",
            "# top\n[theme]\nmode = \"dark\"            # \"dark\" or \"light\"\nblur = 24\n",
        );
        let mut s = NordlysToml::load_from(p.clone());
        s.set("theme", "mode", "light").unwrap();
        s.set("stage", "split", tidy(0.6)).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# top\n"));
        assert!(text.contains("mode = \"light\"            # \"dark\" or \"light\""));
        assert!(text.contains("split = 0.6\n"));
        assert_eq!(s.config.theme.mode, nordlys_config::Mode::Light);
    }

    #[test]
    fn bad_values_are_refused_and_nothing_is_written() {
        let p = temp("bad", "[stage]\nsplit = 0.6\n");
        let mut s = NordlysToml::load_from(p.clone());
        assert!(s.set("stage", "split", 0.95).is_err());
        assert!(s.set("theme", "accent", "blue").is_err());
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "[stage]\nsplit = 0.6\n"
        );
        assert!((s.config.stage.split - 0.6).abs() < 1e-6);
    }

    #[test]
    fn groups_round_trip() {
        let p = temp(
            "groups",
            "[theme]\nmode = \"dark\"\n\n# App groups\n[[group]]\nname = \"Old\"\napps = [\"foot\"]\n",
        );
        let mut s = NordlysToml::load_from(p.clone());
        let g = Group {
            name: "Dev".into(),
            key: Some('d'),
            apps: vec!["helix".into(), "foot".into()],
            open_in: OpenIn::Current,
            dir: Some("~/code".into()),
            reuse_running: false,
        };
        s.set_groups(std::slice::from_ref(&g)).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(
            text.contains("# App groups\n[[group]]\nname = \"Dev\""),
            "{text}"
        );
        assert_eq!(s.config.groups, [g]);
        s.set_groups(&[]).unwrap();
        assert!(s.config.groups.is_empty());
    }

    #[test]
    fn a_broken_file_is_not_overwritten() {
        let p = temp("broken", "[theme]\nacent = 1\n");
        let mut s = NordlysToml::load_from(p.clone());
        assert!(s.problem.is_some());
        assert!(s.set("theme", "mode", "light").is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[theme]\nacent = 1\n");
    }
}
