use crate::app::BlockId;
use crate::app::slint_gen;

use std::cell::RefCell;

use slint::SharedString;
#[derive(Clone)]
pub struct PaletteEntry {
    pub id: BlockId,
    pub name: SharedString,
    pub new_color: slint::Color,
    pub color: slint::Color,
    pub is_mapped: bool,
}

#[derive(Default)]
struct PaletteModelInner {
    filter: BlockFilter,
    entries: Vec<PaletteEntry>,
    indices: Vec<EntryRef>,
}

#[derive(Default)]
pub struct PaletteModel {
    inner: RefCell<PaletteModelInner>,

    notify: slint::ModelNotify,
}

impl PaletteModel {
    /// applies any pending changes, invokes f(id, col) for any (id, col) where col differs from the
    /// color initially associated with id (in PaletteModel::load)
    ///
    /// f may not call any methods on self (will double-borrow the RefCell and panic)
    ///
    /// TODO: i do not really like this, but i also don't like exposing the internal fields
    pub fn apply_changes(&self, mut f: impl FnMut(BlockId, slint::Color)) {
        let mut i = self.inner.borrow_mut();
        for e in i.entries.iter_mut() {
            if e.new_color != e.color {
                e.color = e.new_color;
                e.is_mapped = true;
                f(e.id, e.color)
            }
        }
    }

    pub fn load(&self, it: impl Iterator<Item = PaletteEntry>) {
        self.inner.borrow_mut().load(it);
    }

    pub fn discard_changes(&self) {
        self.inner.borrow_mut().discard_changes();
    }

    pub fn set_color_if_id_present(&self, id: BlockId, col: slint::Color) {
        self.inner.borrow_mut().set_color_if_id_present(id, col);
    }

    pub fn reset_color(&self, id: BlockId) -> Option<slint::Color> {
        self.inner.borrow_mut().reset_color(id)
    }

    pub fn set_pattern(&self, s: &str) {
        let mut i = self.inner.borrow_mut();
        i.set_pattern(s);
        i.rescore();
    }

    pub fn set_filter_group(&self, g: slint_gen::FilterGroup) {
        let mut i = self.inner.borrow_mut();
        i.set_filter_group(g);
        i.rescore();
    }

    pub fn set_filter_status(&self, s: slint_gen::FilterStatus) {
        let mut i = self.inner.borrow_mut();
        i.set_filter_status(s);
        i.rescore();
    }

    // TODO: currently just calling notify.reset() on any change, could do dirty tracking
    pub fn notify_all(&self) {
        self.notify.reset();
    }
}

struct EntryRef {
    idx: usize,
    score: u32,
}

impl PaletteModelInner {
    fn load(&mut self, it: impl Iterator<Item = PaletteEntry>) {
        let n = self.entries.len();
        for entry in it {
            if self.entries[..n]
                .binary_search_by_key(&entry.id, |e| e.id)
                .is_err()
            {
                self.entries.push(entry)
            }
        }
        self.entries.sort_by_key(|e| e.id);
        self.rescore();
    }

    fn discard_changes(&mut self) {
        for entry in self.entries.iter_mut() {
            entry.new_color = entry.color
        }
        self.rescore();
    }

    fn set_color_if_id_present(&mut self, id: BlockId, col: slint::Color) {
        if let Ok(idx) = self.entries.binary_search_by_key(&id, |e| e.id) {
            self.entries[idx].new_color = col;
        }
        self.rescore();
    }

    fn reset_color(&mut self, id: BlockId) -> Option<slint::Color> {
        if let Ok(idx) = self.entries.binary_search_by_key(&id, |e| e.id) {
            self.entries[idx].new_color = self.entries[idx].color;
            self.rescore();
            Some(self.entries[idx].new_color)
        } else {
            None
        }
    }

    fn set_pattern(&mut self, s: &str) {
        self.filter.set_pattern(s);
        self.rescore();
    }

    fn set_filter_group(&mut self, g: slint_gen::FilterGroup) {
        self.filter.set_filter_group(g);
        self.rescore();
    }

    fn set_filter_status(&mut self, s: slint_gen::FilterStatus) {
        self.filter.set_filter_status(s);
        self.rescore();
    }

    fn rescore(&mut self) {
        self.indices.clear();

        self.indices
            .extend(self.entries.iter().enumerate().filter_map(|(idx, e)| {
                Some(EntryRef {
                    idx,
                    score: self.filter.score(e)?,
                })
            }));

        self.indices
            .sort_by(|r1, r2| r2.score.cmp(&r1.score).then(r2.idx.cmp(&r1.idx)));
    }
}

impl slint::Model for PaletteModel {
    type Data = slint_gen::PaletteEntry;
    fn row_count(&self) -> usize {
        self.inner.borrow().indices.len()
    }
    fn row_data(&self, row: usize) -> Option<Self::Data> {
        let inner = self.inner.borrow();
        inner.indices.get(row).map(|i| {
            let e = &inner.entries[i.idx];

            slint_gen::PaletteEntry {
                color: e.new_color,
                id: (u16::from(e.id) as i32),
                name: e.name.clone(),
                is_dirty: e.new_color != e.color,
                is_mapped: e.is_mapped,
            }
        })
    }

    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        &self.notify
    }
}

#[derive(Default)]
struct BlockFilter {
    group: slint_gen::FilterGroup,
    status: slint_gen::FilterStatus,
    matcher: nucleo_matcher::Matcher,
    pattern: nucleo_matcher::pattern::Pattern,
    scratch: Vec<char>,
}

impl BlockFilter {
    fn set_pattern(&mut self, p: &str) {
        self.pattern = nucleo_matcher::pattern::Pattern::new(
            p,
            nucleo_matcher::pattern::CaseMatching::Ignore,
            nucleo_matcher::pattern::Normalization::Never,
            nucleo_matcher::pattern::AtomKind::Fuzzy,
        );
    }

    fn set_filter_group(&mut self, f: slint_gen::FilterGroup) {
        self.group = f;
    }

    fn set_filter_status(&mut self, s: slint_gen::FilterStatus) {
        self.status = s;
    }

    fn score(&mut self, e: &PaletteEntry) -> Option<u32> {
        match self.group {
            slint_gen::FilterGroup::Unmapped if e.is_mapped => return None,
            slint_gen::FilterGroup::Mapped if !e.is_mapped => return None,
            _ => {}
        }

        match self.status {
            slint_gen::FilterStatus::Modified if e.color == e.new_color => return None,
            slint_gen::FilterStatus::Unmodified if e.color != e.new_color => return None,
            _ => {}
        }

        self.pattern.score(
            nucleo_matcher::Utf32Str::new(&e.name, &mut self.scratch),
            &mut self.matcher,
        )
    }
}
