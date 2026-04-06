//! Code snippet storage: CRUD operations and search for reusable code snippets.

use crate::code_gen::Language;
use std::collections::HashMap;

// ── Snippet ───────────────────────────────────────────────────────────

/// A stored code snippet.
#[derive(Debug, Clone)]
pub struct Snippet {
    pub id: String,
    pub title: String,
    pub language: Language,
    pub code: String,
    pub tags: Vec<String>,
    pub created_at: u64,
}

impl Snippet {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        language: Language,
        code: impl Into<String>,
        tags: Vec<String>,
        created_at: u64,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            language,
            code: code.into(),
            tags,
            created_at,
        }
    }

    /// Check whether the snippet matches a search query (case-insensitive).
    #[must_use]
    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.title.to_lowercase().contains(&q)
            || self.code.to_lowercase().contains(&q)
            || self.tags.iter().any(|t| t.to_lowercase().contains(&q))
    }
}

// ── Snippet store ─────────────────────────────────────────────────────

/// In-memory snippet store with CRUD and search.
#[derive(Debug, Clone, Default)]
pub struct SnippetStore {
    snippets: HashMap<String, Snippet>,
    next_id: u64,
}

impl SnippetStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            snippets: HashMap::new(),
            next_id: 1,
        }
    }

    /// Add a snippet, returning a clone of the stored snippet (with auto-generated id).
    pub fn add(
        &mut self,
        title: impl Into<String>,
        language: Language,
        code: impl Into<String>,
        tags: Vec<String>,
        created_at: u64,
    ) -> Snippet {
        let id = self.next_id.to_string();
        self.next_id += 1;
        let snippet = Snippet::new(id.clone(), title, language, code, tags, created_at);
        self.snippets.insert(id, snippet.clone());
        snippet
    }

    /// Get a snippet by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Snippet> {
        self.snippets.get(id)
    }

    /// Remove a snippet by id. Returns the removed snippet if it existed.
    pub fn remove(&mut self, id: &str) -> Option<Snippet> {
        self.snippets.remove(id)
    }

    /// Update a snippet's title and tags.
    pub fn update(&mut self, id: &str, title: Option<String>, tags: Option<Vec<String>>) -> bool {
        if let Some(snippet) = self.snippets.get_mut(id) {
            if let Some(t) = title {
                snippet.title = t;
            }
            if let Some(tg) = tags {
                snippet.tags = tg;
            }
            true
        } else {
            false
        }
    }

    /// List all snippets, sorted by `created_at` descending (newest first).
    #[must_use]
    pub fn list(&self) -> Vec<&Snippet> {
        let mut items: Vec<&Snippet> = self.snippets.values().collect();
        items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        items
    }

    /// List snippets for a specific language.
    #[must_use]
    pub fn for_language(&self, lang: Language) -> Vec<&Snippet> {
        self.snippets
            .values()
            .filter(|s| s.language == lang)
            .collect()
    }

    /// Search snippets by query string (matches title, code, and tags).
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&Snippet> {
        self.snippets
            .values()
            .filter(|s| s.matches(query))
            .collect()
    }

    /// Search snippets by tag.
    #[must_use]
    pub fn search_by_tag(&self, tag: &str) -> Vec<&Snippet> {
        let t = tag.to_lowercase();
        self.snippets
            .values()
            .filter(|s| s.tags.iter().any(|st| st.to_lowercase() == t))
            .collect()
    }

    /// Number of stored snippets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.snippets.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.snippets.is_empty()
    }
}

/// Convenience: search snippets in a store.
#[must_use]
pub fn search_snippets<'a>(store: &'a SnippetStore, query: &str) -> Vec<&'a Snippet> {
    store.search(query)
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_new() {
        let s = Snippet::new(
            "1",
            "hello",
            Language::Rust,
            "fn main() {}",
            vec!["entry".into()],
            100,
        );
        assert_eq!(s.id, "1");
        assert_eq!(s.title, "hello");
        assert_eq!(s.language, Language::Rust);
        assert!(!s.code.is_empty());
    }

    #[test]
    fn snippet_matches_title() {
        let s = Snippet::new("1", "Sorting Algorithm", Language::Python, "", vec![], 0);
        assert!(s.matches("sorting"));
        assert!(s.matches("ALGORITHM"));
        assert!(!s.matches("binary"));
    }

    #[test]
    fn snippet_matches_code() {
        let s = Snippet::new("1", "test", Language::Rust, "fn bubble_sort()", vec![], 0);
        assert!(s.matches("bubble"));
    }

    #[test]
    fn snippet_matches_tags() {
        let s = Snippet::new("1", "test", Language::Go, "", vec!["concurrency".into()], 0);
        assert!(s.matches("concurrency"));
        assert!(!s.matches("sorting"));
    }

    #[test]
    fn store_add_and_get() {
        let mut store = SnippetStore::new();
        let s = store.add("Test", Language::Rust, "code", vec![], 100);
        assert_eq!(s.id, "1");
        assert!(store.get("1").is_some());
        assert!(store.get("99").is_none());
    }

    #[test]
    fn store_auto_increment_ids() {
        let mut store = SnippetStore::new();
        let s1 = store.add("A", Language::Rust, "", vec![], 0);
        let s2 = store.add("B", Language::Go, "", vec![], 1);
        assert_eq!(s1.id, "1");
        assert_eq!(s2.id, "2");
    }

    #[test]
    fn store_remove() {
        let mut store = SnippetStore::new();
        store.add("Test", Language::Rust, "code", vec![], 0);
        assert_eq!(store.len(), 1);
        let removed = store.remove("1");
        assert!(removed.is_some());
        assert_eq!(store.len(), 0);
        assert!(store.remove("1").is_none());
    }

    #[test]
    fn store_update() {
        let mut store = SnippetStore::new();
        store.add("Old Title", Language::Rust, "code", vec!["old".into()], 0);
        assert!(store.update("1", Some("New Title".into()), None));
        assert_eq!(store.get("1").unwrap().title, "New Title");
        assert!(store.update("1", None, Some(vec!["new".into()])));
        assert_eq!(store.get("1").unwrap().tags, vec!["new"]);
        assert!(!store.update("99", Some("X".into()), None));
    }

    #[test]
    fn store_list_sorted_by_created_at() {
        let mut store = SnippetStore::new();
        store.add("Old", Language::Rust, "", vec![], 10);
        store.add("New", Language::Rust, "", vec![], 50);
        store.add("Mid", Language::Rust, "", vec![], 30);
        let list = store.list();
        assert_eq!(list[0].title, "New");
        assert_eq!(list[1].title, "Mid");
        assert_eq!(list[2].title, "Old");
    }

    #[test]
    fn store_for_language() {
        let mut store = SnippetStore::new();
        store.add("Rs1", Language::Rust, "", vec![], 0);
        store.add("Py1", Language::Python, "", vec![], 1);
        store.add("Rs2", Language::Rust, "", vec![], 2);
        assert_eq!(store.for_language(Language::Rust).len(), 2);
        assert_eq!(store.for_language(Language::Python).len(), 1);
        assert_eq!(store.for_language(Language::Go).len(), 0);
    }

    #[test]
    fn store_search() {
        let mut store = SnippetStore::new();
        store.add(
            "Bubble Sort",
            Language::Python,
            "def bubble_sort():",
            vec!["sorting".into()],
            0,
        );
        store.add(
            "Binary Search",
            Language::Rust,
            "fn binary_search()",
            vec!["search".into()],
            1,
        );
        assert_eq!(store.search("sort").len(), 1);
        assert_eq!(store.search("binary").len(), 1);
        assert_eq!(store.search("xyz").len(), 0);
    }

    #[test]
    fn store_search_by_tag() {
        let mut store = SnippetStore::new();
        store.add(
            "A",
            Language::Rust,
            "",
            vec!["algo".into(), "sort".into()],
            0,
        );
        store.add("B", Language::Rust, "", vec!["algo".into()], 1);
        assert_eq!(store.search_by_tag("algo").len(), 2);
        assert_eq!(store.search_by_tag("sort").len(), 1);
        assert_eq!(store.search_by_tag("ALGO").len(), 2);
    }

    #[test]
    fn store_is_empty() {
        let store = SnippetStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn search_snippets_convenience() {
        let mut store = SnippetStore::new();
        store.add(
            "Hello World",
            Language::JavaScript,
            "console.log('hi')",
            vec![],
            0,
        );
        let results = search_snippets(&store, "hello");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn store_default() {
        let store = SnippetStore::default();
        assert!(store.is_empty());
    }
}
