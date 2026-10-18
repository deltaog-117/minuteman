// Minuteman - a fast, Ranger-inspired terminal file manager
// Copyright (C) 2026  Davi Oliveira Gonçalves
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Which plugin answers which `:` command word. The words a plugin lists under `commands` in
//! config and the ones it registers while running (`register_command`) live in one place, so
//! there is a single answer to "who owns `:blame`", and a plugin's request is answered from it at
//! once, on the task that read it, without waiting for the render thread.
//!
//! A word is kept after the process that registered it has ended, since a plugin that finished is
//! started again by the next `:` command meant for it and so must still be found. Only a restart
//! (`:plugins restart`) forgets a plugin's runtime words, as the fresh process registers again.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The registry as the host and every plugin's reader share it.
#[derive(Debug, Default, Clone)]
pub struct SharedRegistry(Arc<Mutex<Registry>>);

impl SharedRegistry {
    pub fn lock(&self) -> MutexGuard<'_, Registry> {
        // Nothing here panics while holding the lock, and a poisoned registry is still a valid
        // one, so carry on rather than take every plugin's reader down with it.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Why a word could not be registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// Not a word a `:` line can start with.
    Invalid,
    /// A built-in command, which always wins.
    Reserved,
    /// Another plugin has it.
    Taken { by: String },
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => {
                f.write_str("a command name is one or more letters, digits, '-', '_', '.' or '+'")
            }
            Self::Reserved => f.write_str("that is a built-in command"),
            Self::Taken { by } => write!(f, "that command already belongs to plugin '{by}'"),
        }
    }
}

/// Why a word could not be unregistered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnregisterError {
    /// This plugin has no such word, or it is another plugin's.
    NotYours,
    /// It was declared in config, so only the config can take it away.
    FromConfig,
}

impl fmt::Display for UnregisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotYours => f.write_str("that command is not registered by this plugin"),
            Self::FromConfig => f.write_str("that command is declared in the config file"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Owner {
    plugin: String,
    /// Registered while running, as opposed to declared in config.
    runtime: bool,
}

#[derive(Debug, Default)]
pub struct Registry {
    owners: HashMap<String, Owner>,
    reserved: HashSet<String>,
}

/// Whether `word` can be registered at run time: something a `:` line can start with and reach a
/// plugin, so no spaces and none of the characters that make a line shell syntax.
fn is_valid_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
}

impl Registry {
    /// Words no plugin may claim, because the interface has a command by that name.
    pub fn reserve(&mut self, words: impl IntoIterator<Item = String>) {
        self.reserved.extend(words);
    }

    /// Gives `word` to `plugin`. A word from config (`runtime == false`) is held to the same
    /// ownership rules, but not to the character rule, which config never had. Asking again for a
    /// word the plugin already holds is fine.
    pub fn register(
        &mut self,
        plugin: &str,
        word: &str,
        runtime: bool,
    ) -> Result<(), RegisterError> {
        if runtime && !is_valid_word(word) {
            return Err(RegisterError::Invalid);
        }
        if self.reserved.contains(word) {
            return Err(RegisterError::Reserved);
        }
        match self.owners.get_mut(word) {
            Some(owner) if owner.plugin != plugin => Err(RegisterError::Taken {
                by: owner.plugin.clone(),
            }),
            // Declaring a word in config that was registered while running makes it the config's.
            Some(owner) => {
                owner.runtime &= runtime;
                Ok(())
            }
            None => {
                self.owners.insert(
                    word.to_string(),
                    Owner {
                        plugin: plugin.to_string(),
                        runtime,
                    },
                );
                Ok(())
            }
        }
    }

    /// Takes back a word `plugin` registered while running.
    pub fn unregister(&mut self, plugin: &str, word: &str) -> Result<(), UnregisterError> {
        match self.owners.get(word) {
            Some(owner) if owner.plugin == plugin && owner.runtime => {
                self.owners.remove(word);
                Ok(())
            }
            Some(owner) if owner.plugin == plugin => Err(UnregisterError::FromConfig),
            _ => Err(UnregisterError::NotYours),
        }
    }

    /// Forgets what `plugin` registered while running, keeping the words its config declared.
    pub fn clear_runtime(&mut self, plugin: &str) {
        self.owners
            .retain(|_, owner| owner.plugin != plugin || !owner.runtime);
    }

    /// Forgets every word `plugin` holds, config and runtime alike, for a plugin that is taken
    /// out of the session rather than started again.
    pub fn forget(&mut self, plugin: &str) {
        self.owners.retain(|_, owner| owner.plugin != plugin);
    }

    /// The plugin that answers `word`, if any.
    pub fn owner(&self, word: &str) -> Option<&str> {
        self.owners.get(word).map(|owner| owner.plugin.as_str())
    }

    /// Every word `plugin` answers, config and runtime, sorted.
    pub fn words_of(&self, plugin: &str) -> Vec<String> {
        let mut words: Vec<String> = self
            .owners
            .iter()
            .filter(|(_, owner)| owner.plugin == plugin)
            .map(|(word, _)| word.clone())
            .collect();
        words.sort();
        words
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn registry() -> Registry {
        let mut registry = Registry::default();
        registry.reserve(["cd".to_string(), "q".to_string()]);
        registry
    }

    #[test]
    fn forgetting_a_plugin_frees_its_config_and_runtime_words_and_nobody_else_s() {
        let mut registry = registry();
        registry.register("a", "blame", false).unwrap();
        registry.register("a", "log", true).unwrap();
        registry.register("b", "stats", false).unwrap();

        registry.forget("a");

        assert_eq!(registry.owner("blame"), None);
        assert_eq!(registry.owner("log"), None);
        assert_eq!(registry.owner("stats"), Some("b"));
        assert_eq!(registry.register("b", "blame", false), Ok(()));
    }

    #[test]
    fn a_free_word_goes_to_the_first_plugin_to_ask() {
        let mut registry = registry();
        assert_eq!(registry.register("a", "blame", true), Ok(()));
        assert_eq!(registry.owner("blame"), Some("a"));
        assert_eq!(registry.owner("other"), None);
    }

    #[test]
    fn asking_again_for_a_word_you_hold_is_fine() {
        let mut registry = registry();
        registry.register("a", "blame", true).unwrap();
        assert_eq!(registry.register("a", "blame", true), Ok(()));
        assert_eq!(registry.words_of("a"), ["blame"]);
    }

    #[test]
    fn a_word_another_plugin_holds_is_refused_and_stays_where_it_was() {
        let mut registry = registry();
        registry.register("a", "blame", true).unwrap();
        assert_eq!(
            registry.register("b", "blame", true),
            Err(RegisterError::Taken { by: "a".into() })
        );
        assert_eq!(registry.owner("blame"), Some("a"));
    }

    #[test]
    fn a_built_in_word_is_refused_from_config_and_from_a_running_plugin() {
        let mut registry = registry();
        assert_eq!(
            registry.register("a", "cd", true),
            Err(RegisterError::Reserved)
        );
        assert_eq!(
            registry.register("a", "q", false),
            Err(RegisterError::Reserved)
        );
        assert_eq!(registry.owner("cd"), None);
    }

    #[test]
    fn a_running_plugin_may_not_register_what_a_command_line_could_not_deliver() {
        let mut registry = registry();
        for word in [
            "",
            "two words",
            "a|b",
            "a;b",
            "$x",
            "*",
            "a b",
            "tab\t",
            "-é!",
        ] {
            assert_eq!(
                registry.register("a", word, true),
                Err(RegisterError::Invalid),
                "{word:?}"
            );
        }
        for word in ["blame", "git-blame", "dir_stats", "v1.2", "c++", "été"] {
            assert_eq!(registry.register("a", word, true), Ok(()), "{word:?}");
        }
    }

    #[test]
    fn config_words_keep_the_looser_rule_they_always_had() {
        let mut registry = registry();
        assert_eq!(registry.register("a", "odd!word", false), Ok(()));
        assert_eq!(registry.owner("odd!word"), Some("a"));
    }

    #[test]
    fn only_a_word_registered_while_running_can_be_unregistered_by_its_owner() {
        let mut registry = registry();
        registry.register("a", "live", true).unwrap();
        registry.register("a", "fixed", false).unwrap();

        assert_eq!(
            registry.unregister("b", "live"),
            Err(UnregisterError::NotYours)
        );
        assert_eq!(
            registry.unregister("a", "nothing"),
            Err(UnregisterError::NotYours)
        );
        assert_eq!(
            registry.unregister("a", "fixed"),
            Err(UnregisterError::FromConfig)
        );
        assert_eq!(registry.unregister("a", "live"), Ok(()));
        assert_eq!(registry.owner("live"), None);
        assert_eq!(registry.owner("fixed"), Some("a"));
    }

    #[test]
    fn a_word_freed_by_unregistering_can_go_to_someone_else() {
        let mut registry = registry();
        registry.register("a", "blame", true).unwrap();
        registry.unregister("a", "blame").unwrap();
        assert_eq!(registry.register("b", "blame", true), Ok(()));
        assert_eq!(registry.owner("blame"), Some("b"));
    }

    #[test]
    fn clearing_forgets_running_words_and_keeps_config_words_and_other_plugins() {
        let mut registry = registry();
        registry.register("a", "live", true).unwrap();
        registry.register("a", "fixed", false).unwrap();
        registry.register("b", "theirs", true).unwrap();

        registry.clear_runtime("a");

        assert_eq!(registry.words_of("a"), ["fixed"]);
        assert_eq!(registry.words_of("b"), ["theirs"]);
    }

    #[test]
    fn declaring_a_word_in_config_that_was_registered_while_running_makes_it_the_configs() {
        let mut registry = registry();
        registry.register("a", "both", true).unwrap();
        registry.register("a", "both", false).unwrap();
        assert_eq!(
            registry.unregister("a", "both"),
            Err(UnregisterError::FromConfig)
        );
    }

    #[test]
    fn every_error_says_what_to_do_about_it() {
        assert!(
            RegisterError::Invalid
                .to_string()
                .contains("letters, digits")
        );
        assert!(RegisterError::Reserved.to_string().contains("built-in"));
        assert!(
            RegisterError::Taken { by: "x".into() }
                .to_string()
                .contains("'x'")
        );
    }

    #[derive(Debug, Clone)]
    enum Op {
        Register {
            plugin: usize,
            word: usize,
            runtime: bool,
        },
        Unregister {
            plugin: usize,
            word: usize,
        },
        Clear {
            plugin: usize,
        },
    }

    fn ops() -> impl Strategy<Value = Vec<Op>> {
        let op = prop_oneof![
            (0usize..3, 0usize..5, any::<bool>()).prop_map(|(plugin, word, runtime)| {
                Op::Register {
                    plugin,
                    word,
                    runtime,
                }
            }),
            (0usize..3, 0usize..5).prop_map(|(plugin, word)| Op::Unregister { plugin, word }),
            (0usize..3).prop_map(|plugin| Op::Clear { plugin }),
        ];
        proptest::collection::vec(op, 1..60)
    }

    proptest! {
        /// Whatever the order of requests, the registry agrees with a plain model of the rules:
        /// a word has at most one owner, an owner only changes hands once the word is freed, a
        /// built-in is never owned, and clearing a plugin leaves its config words alone.
        #[test]
        fn the_registry_follows_the_ownership_rules_for_any_sequence_of_requests(ops in ops()) {
            const PLUGINS: [&str; 3] = ["p0", "p1", "p2"];
            const WORDS: [&str; 5] = ["w0", "w1", "w2", "cd", "w4"];
            let mut registry = registry();
            // word -> (owner, runtime)
            let mut model: HashMap<&str, (&str, bool)> = HashMap::new();

            for op in ops {
                match op {
                    Op::Register { plugin, word, runtime } => {
                        let (plugin, word) = (PLUGINS[plugin], WORDS[word]);
                        let got = registry.register(plugin, word, runtime);
                        let expected = if word == "cd" {
                            Err(RegisterError::Reserved)
                        } else {
                            match model.get_mut(word) {
                                Some((owner, _)) if *owner != plugin => {
                                    Err(RegisterError::Taken { by: (*owner).to_string() })
                                }
                                Some((_, was_runtime)) => {
                                    *was_runtime &= runtime;
                                    Ok(())
                                }
                                None => {
                                    model.insert(word, (plugin, runtime));
                                    Ok(())
                                }
                            }
                        };
                        prop_assert_eq!(got, expected);
                    }
                    Op::Unregister { plugin, word } => {
                        let (plugin, word) = (PLUGINS[plugin], WORDS[word]);
                        let got = registry.unregister(plugin, word);
                        let expected = match model.get(word) {
                            Some((owner, true)) if *owner == plugin => {
                                model.remove(word);
                                Ok(())
                            }
                            Some((owner, false)) if *owner == plugin => {
                                Err(UnregisterError::FromConfig)
                            }
                            _ => Err(UnregisterError::NotYours),
                        };
                        prop_assert_eq!(got, expected);
                    }
                    Op::Clear { plugin } => {
                        let plugin = PLUGINS[plugin];
                        registry.clear_runtime(plugin);
                        model.retain(|_, (owner, runtime)| *owner != plugin || !*runtime);
                    }
                }
                for word in WORDS {
                    prop_assert_eq!(registry.owner(word), model.get(word).map(|(owner, _)| *owner));
                }
                prop_assert_eq!(registry.owner("cd"), None);
            }
        }
    }
}
