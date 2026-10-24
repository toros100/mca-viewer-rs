use crate::app::WorldInfo;
use crate::app::slint_gen;
use slint::ToSharedString;
use std::cell::RefCell;

#[derive(Default)]
pub struct WorldsModel {
    inner: RefCell<WorldsModelInner>,
    notify: slint::ModelNotify,
}

#[derive(Default)]
struct WorldsModelInner {
    worlds: Vec<WorldInfo>,
    model_data: Vec<slint_gen::WorldInfo>,
}

impl WorldsModel {
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    pub fn load(&self, v: Vec<WorldInfo>) {
        self.inner.borrow_mut().load(v);
    }

    pub fn get(&self, i: usize) -> Option<WorldInfo> {
        self.inner.borrow().worlds.get(i).cloned()
    }

    pub fn notify_all(&self) {
        self.notify.reset();
    }
}

impl WorldsModelInner {
    #[expect(unused)]
    fn clear(&mut self) {
        self.worlds.clear();
        self.model_data.clear();
    }

    fn load(&mut self, v: Vec<WorldInfo>) {
        self.worlds = v;
        self.worlds
            .sort_by(|w1, w2| w2.level_dat.last_played.cmp(&w1.level_dat.last_played));

        self.model_data.clear();

        let tz = jiff::tz::TimeZone::system();
        self.model_data.extend(self.worlds.iter().map(|w| {
            let zoned_time = w.level_dat.last_played.to_zoned(tz.clone());

            let last_played = match jiff::fmt::rfc2822::to_string(&zoned_time) {
                Ok(l) => l.to_shared_string(),
                Err(_) => slint::format!("unknown"),
            };

            slint_gen::WorldInfo {
                last_played,
                name: w.level_dat.level_name.to_shared_string(),
                end_regions: w.end_regions.as_ref().map_or(-1, |r| r.count as i32),
                overworld_regions: w.overworld_regions.as_ref().map_or(-1, |r| r.count as i32),
                nether_regions: w.nether_regions.as_ref().map_or(-1, |r| r.count as i32),
                version: w.level_dat.version.name.to_shared_string(),
            }
        }));
    }
}

impl slint::Model for WorldsModel {
    type Data = slint_gen::WorldInfo;
    fn row_count(&self) -> usize {
        self.inner.borrow().worlds.len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        self.inner.borrow().model_data.get(row).cloned()
    }

    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        &self.notify
    }
}
