//! Tab hibernation policy.
//!
//! Idle background tabs are "frozen": their URL, title, scroll position and a
//! screenshot are written to SQLite and the renderer is destroyed, which gives
//! the memory back to the system. Selecting the tab loads it again. This
//! module decides *which* tabs to hibernate; `manager` does the work.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HibernationPolicy {
    pub enabled: bool,
    /// Minutes a background tab may sit unused before it is hibernated.
    pub idle_minutes: u32,
    /// Same, while a game is running.
    pub game_mode_idle_minutes: u32,
    /// Most tabs kept loaded at once (0 = no limit).
    pub max_live_tabs: usize,
    /// Same, while a game is running.
    pub game_mode_max_live_tabs: usize,
    pub keep_pinned: bool,
    pub keep_audible: bool,
}

impl Default for HibernationPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            idle_minutes: 15,
            game_mode_idle_minutes: 1,
            max_live_tabs: 12,
            game_mode_max_live_tabs: 2,
            keep_pinned: true,
            keep_audible: true,
        }
    }
}

/// What the policy needs to know about a loaded tab.
#[derive(Debug, Clone, PartialEq)]
pub struct TabActivity {
    pub id: String,
    /// Shown in a pane right now.
    pub visible: bool,
    pub pinned: bool,
    pub audible: bool,
    pub last_active: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pressure {
    /// A fullscreen game is running.
    pub game_active: bool,
    /// Memory use is close to the user's limit: hibernate everything eligible.
    pub memory_critical: bool,
}

impl HibernationPolicy {
    fn protected(&self, tab: &TabActivity) -> bool {
        tab.visible || (self.keep_pinned && tab.pinned) || (self.keep_audible && tab.audible)
    }

    /// Ids of the tabs to hibernate now, least recently used first.
    pub fn select(&self, now: Instant, tabs: &[TabActivity], pressure: Pressure) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let mut candidates: Vec<&TabActivity> = tabs.iter().filter(|tab| !self.protected(tab)).collect();
        candidates.sort_by_key(|tab| tab.last_active);

        if pressure.memory_critical {
            return candidates.into_iter().map(|tab| tab.id.clone()).collect();
        }

        let (idle_minutes, max_live) = if pressure.game_active {
            (self.game_mode_idle_minutes, self.game_mode_max_live_tabs)
        } else {
            (self.idle_minutes, self.max_live_tabs)
        };
        let idle_limit = Duration::from_secs(u64::from(idle_minutes) * 60);
        // Tabs over the live-tab budget go first, oldest first.
        let over_budget = if max_live == 0 { 0 } else { tabs.len().saturating_sub(max_live) };

        candidates
            .into_iter()
            .enumerate()
            .filter(|(index, tab)| *index < over_budget || now.saturating_duration_since(tab.last_active) >= idle_limit)
            .map(|(_, tab)| tab.id.clone())
            .collect()
    }
}

/// JavaScript run in a tab just before hibernation; resolves to the state to
/// restore later.
pub const CAPTURE_STATE_SCRIPT: &str = "JSON.stringify({scrollY: window.scrollY || 0})";

/// JavaScript that restores the scroll position after the tab reloads.
pub fn restore_scroll_script(scroll_y: f64) -> String {
    if scroll_y <= 0.0 || !scroll_y.is_finite() {
        return String::new();
    }
    format!(
        "(function(){{var y={scroll_y};function go(){{window.scrollTo(0,y);}}\
if(document.readyState==='complete')go();else window.addEventListener('load',go,{{once:true}});}})();"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str, idle_minutes: u64, now: Instant) -> TabActivity {
        TabActivity {
            id: id.into(),
            visible: false,
            pinned: false,
            audible: false,
            last_active: now - Duration::from_secs(idle_minutes * 60),
        }
    }

    fn now() -> Instant {
        Instant::now() + Duration::from_secs(100_000)
    }

    #[test]
    fn hibernates_idle_tabs_and_protects_important_ones() {
        let now = now();
        let policy = HibernationPolicy::default();
        let tabs = vec![
            TabActivity { visible: true, ..tab("visible", 60, now) },
            TabActivity { pinned: true, ..tab("pinned", 60, now) },
            TabActivity { audible: true, ..tab("music", 60, now) },
            tab("old", 30, now),
            tab("older", 90, now),
            tab("fresh", 2, now),
        ];
        assert_eq!(policy.select(now, &tabs, Pressure::default()), vec!["older", "old"]);
    }

    #[test]
    fn game_mode_is_aggressive() {
        let now = now();
        let policy = HibernationPolicy::default();
        let tabs = vec![
            TabActivity { visible: true, ..tab("visible", 0, now) },
            tab("a", 2, now),
            tab("b", 5, now),
        ];
        assert!(policy.select(now, &tabs, Pressure::default()).is_empty());
        let game = Pressure { game_active: true, memory_critical: false };
        assert_eq!(policy.select(now, &tabs, game), vec!["b", "a"]);
    }

    #[test]
    fn live_tab_budget_evicts_least_recently_used() {
        let now = now();
        let policy = HibernationPolicy { max_live_tabs: 3, ..Default::default() };
        let tabs: Vec<TabActivity> = (0..5).map(|i| tab(&format!("t{i}"), i + 1, now)).collect();
        // Five loaded, budget three: the two least recently used go.
        assert_eq!(policy.select(now, &tabs, Pressure::default()), vec!["t4", "t3"]);
    }

    #[test]
    fn memory_pressure_takes_everything_eligible() {
        let now = now();
        let policy = HibernationPolicy::default();
        let tabs = vec![TabActivity { visible: true, ..tab("visible", 0, now) }, tab("a", 0, now), tab("b", 1, now)];
        let critical = Pressure { game_active: false, memory_critical: true };
        assert_eq!(policy.select(now, &tabs, critical), vec!["b", "a"]);
        let disabled = HibernationPolicy { enabled: false, ..Default::default() };
        assert!(disabled.select(now, &tabs, critical).is_empty());
    }

    #[test]
    fn scroll_restore_script() {
        assert_eq!(restore_scroll_script(0.0), "");
        assert!(restore_scroll_script(420.0).contains("scrollTo(0,y)"));
    }
}
