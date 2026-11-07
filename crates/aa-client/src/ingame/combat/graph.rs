//! Blend times of action clips, from the 1.2 animation graph (`animations/graphs/player_full_body.xml`).
//!
//! x2game never picks a blend: it sets the graph's `Signal` input, and CryAction's AnimationGraph starts the clip of the state it
//! reaches with that state's own transition time, replaced by the largest `<Link transitionTime>` walked since the last state
//! that played an animation when that is above 0 (CryAction `30634510`, `305ED7B0`). Every hub (`Main_Hub`, `Skill Start`,
//! `Skill End`, `Idle Start`) links with 0, so a walk is the exit link of the clip that played (`X_anim -> Skill End`) and the
//! entry link of the next state (`Skill Start -> Y`):
//!
//! * the first clip of a skill blends in 0.01 s (`Skill Start -> [item1]_co_sk_streakattack_1`), a shout 0.2 s;
//! * a clip chained to another 0.2 s, the exit of the one before (Triple Slash 2 and 3, a launch after its cast);
//! * Whirlwind Slash's `twoswords_co_sk_weapon_slash_3` has a 0 entry link: its state's own 0.1 s;
//! * the idle after a clip takes that clip's exit (0.3 s after a shout, 0.2 s after Triple Slash); with none, its own 0.2 s.
//!
//! All measured on the 1.2 client (skill parity trace, 2026-11-03), where aa-client blended every action in 0.08 s and every
//! idle in 0.15 s.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The blend of an idle, and of a state whose template says nothing (`IdleOneShot`, `locomotion_idle`: 0.2).
pub const DEFAULT_BLEND: f32 = 0.2;

/// The blend of a clip on the upper body while the legs run (`player_upper_body.xml`, `ub_one_shot_interruptable`: 0.2).
pub const UPPER_BODY_BLEND: f32 = 0.2;

/// What the graph says about the action states: their entry and exit links and their own transition times.
#[derive(Debug, Default)]
pub struct ActionGraph {
    /// State id -> the largest transition time of the links into it.
    enter: HashMap<String, f32>,
    /// State id -> the largest transition time on the way out of it: its own links, and the links out of a hub one reaches (a
    /// hub plays nothing, so the walk goes on through it: `Action End -> Main_Hub` is 0.5).
    exit: HashMap<String, f32>,
    /// State id of a state that plays an animation -> its own transition time (`transition_time`, else its template's default).
    own: HashMap<String, f32>,
}

impl ActionGraph {
    /// The graph of the game's pak, read once (empty without one: every blend is then the caller's default).
    pub fn get() -> &'static ActionGraph {
        static GRAPH: OnceLock<ActionGraph> = OnceLock::new();
        GRAPH.get_or_init(|| {
            let read = |path: &str| aa_cry::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
            read("animations/graphs/player_full_body.xml")
                .map(|xml| ActionGraph::parse(&xml, |name| read(&format!("libs/animationgraphtemplates/{}.xml", name.to_ascii_lowercase()))))
                .unwrap_or_default()
        })
    }

    /// Reads a graph; `template` gives the XML of an `AGTemplate` by name (for the default transition times).
    pub fn parse(xml: &str, template: impl Fn(&str) -> Option<String>) -> ActionGraph {
        let Ok(doc) = roxmltree::Document::parse(xml) else { return ActionGraph::default() };
        let mut graph = ActionGraph::default();
        let mut defaults: HashMap<String, f32> = HashMap::new();
        for state in doc.descendants().filter(|n| n.has_tag_name("State")) {
            let Some(id) = state.attribute("id") else { continue };
            let Some(t) = state.children().filter(|n| n.has_tag_name("Template")).find(|t| t.attribute("animation").is_some_and(|a| !a.trim().is_empty())) else {
                continue;
            };
            let name = t.attribute("name").unwrap_or_default();
            let own = match t.attribute("transition_time").and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| *v > 0.0) {
                Some(v) => v,
                None => *defaults.entry(name.to_string()).or_insert_with(|| template_default(name, &template, 0)),
            };
            graph.own.insert(id.to_string(), own);
        }
        let links: Vec<(&str, &str, f32)> = doc
            .descendants()
            .filter(|n| n.has_tag_name("Link"))
            .filter_map(|l| Some((l.attribute("from")?, l.attribute("to")?, l.attribute("transitionTime").and_then(|t| t.trim().parse().ok()).unwrap_or(0.0))))
            .collect();
        let mut out: HashMap<&str, f32> = HashMap::new();
        for &(from, to, t) in &links {
            let into = graph.enter.entry(to.to_string()).or_insert(0.0);
            *into = into.max(t);
            let away = out.entry(from).or_insert(0.0);
            *away = away.max(t);
        }
        for &(from, to, t) in &links {
            let through = if graph.own.contains_key(to) { 0.0 } else { out.get(to).copied().unwrap_or(0.0) };
            let exit = graph.exit.entry(from.to_string()).or_insert(0.0);
            *exit = exit.max(t).max(through);
        }
        graph
    }

    /// The state of clip `name`: a state of that name, else the generic one with the weapon part as `[item1]`
    /// (`twohand_co_sk_streakattack_1` -> `[item1]_co_sk_streakattack_1`).
    fn state(&self, name: &str) -> Option<String> {
        let known = |id: &str| self.enter.contains_key(id) || self.own.contains_key(id);
        if known(name) {
            return Some(name.to_string());
        }
        let generic = format!("[item1]{}", &name[name.find('_')?..]);
        known(&generic).then_some(generic)
    }

    /// The graph knows the clip.
    pub fn knows(&self, name: &str) -> bool {
        self.state(name).is_some()
    }

    /// The largest link time on the way into clip `name`'s state.
    pub fn enter(&self, name: &str) -> f32 {
        self.state(name).and_then(|s| self.enter.get(&s).copied()).unwrap_or(0.0)
    }

    /// The largest link time on the way out of clip `name`: its state's and its `_anim` state's links.
    pub fn exit(&self, name: &str) -> f32 {
        let Some(s) = self.state(name) else { return 0.0 };
        let of = |id: &str| self.exit.get(id).copied().unwrap_or(0.0);
        of(&s).max(of(&format!("{s}_anim")))
    }

    /// The own transition time of the state that plays clip `name` (the state, else its `_anim` state).
    pub fn own(&self, name: &str) -> Option<f32> {
        let s = self.state(name)?;
        self.own.get(&s).or_else(|| self.own.get(&format!("{s}_anim"))).copied()
    }

    /// The blend of action clip `name` started `exit` after the clip before it (the exit link of the action the unit is still in,
    /// 0 from an idle or a run). `None` when the graph does not know the clip.
    pub fn action_blend(&self, exit: f32, name: &str) -> Option<f32> {
        self.knows(name).then_some(())?;
        let walked = exit.max(self.enter(name));
        Some(if walked > 0.0 { walked } else { self.own(name).unwrap_or(DEFAULT_BLEND) })
    }

    /// The blend of the idle (or run) that follows an action whose exit link is `exit`.
    pub fn idle_blend(exit: f32) -> f32 {
        if exit > 0.0 { exit } else { DEFAULT_BLEND }
    }
}

/// The default `TransitionTime` of a template's `TransitionParamsLayer1`: a number, or `iff( $transition_time == 0, <default>, ...)`;
/// else the template it extends. [`DEFAULT_BLEND`] when nothing says.
fn template_default(name: &str, template: &impl Fn(&str) -> Option<String>, depth: u8) -> f32 {
    if depth > 6 || name.is_empty() {
        return DEFAULT_BLEND;
    }
    let Some(xml) = template(name) else { return DEFAULT_BLEND };
    let Ok(doc) = roxmltree::Document::parse(&xml) else { return DEFAULT_BLEND };
    if let Some(expr) = doc.descendants().find(|n| n.has_tag_name("TransitionParamsLayer1")).and_then(|n| n.attribute("TransitionTime")) {
        return transition_default(expr).unwrap_or(DEFAULT_BLEND);
    }
    match doc.root_element().attribute("extend") {
        Some(parent) => template_default(parent, template, depth + 1),
        None => DEFAULT_BLEND,
    }
}

/// `0.1` -> 0.1; `iff( $transition_time == 0, 0.2, $transition_time )` -> 0.2.
fn transition_default(expr: &str) -> Option<f32> {
    let expr = expr.trim();
    if let Ok(v) = expr.parse() {
        return Some(v);
    }
    let rest = expr.strip_prefix("iff")?.trim_start().strip_prefix('(')?;
    let mut parts = rest.split(',');
    let condition = parts.next()?.replace(' ', "");
    condition.ends_with("==0").then_some(())?;
    parts.next()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str = r#"<AnimationGraph>
      <States>
        <State id="Skill Start"><Template name="Default"/></State>
        <State id="Skill End"><Template name="Default"/></State>
        <State id="[item1]_co_sk_streakattack_1"><Template name="Default"/></State>
        <State id="[item1]_co_sk_streakattack_1_anim"><Template animation="[item1]_co_sk_streakattack_1" transition_time="" name="IdleOneShotNoLookIk"/></State>
        <State id="all_co_sk_shout"><Template name="Default"/></State>
        <State id="all_co_sk_shout_anim"><Template animation="all_co_sk_shout" transition_time="0.2" name="IdleOneShotNoLookIk"/></State>
        <State id="twoswords_co_sk_weapon_slash_3"><Template name="Default"/></State>
        <State id="twoswords_co_sk_weapon_slash_3_anim"><Template animation="twoswords_co_sk_weapon_slash_3" transition_time="0.1" name="IdleOneShotNoLookIk"/></State>
      </States>
      <Links>
        <Link transitionTime="0.0099999998" from="Skill Start" to="[item1]_co_sk_streakattack_1"/>
        <Link transitionTime="0" from="[item1]_co_sk_streakattack_1" to="[item1]_co_sk_streakattack_1_anim"/>
        <Link transitionTime="0.2" from="[item1]_co_sk_streakattack_1_anim" to="Skill End"/>
        <Link transitionTime="0.2" from="Skill Start" to="all_co_sk_shout"/>
        <Link transitionTime="0" from="all_co_sk_shout" to="all_co_sk_shout_anim"/>
        <Link transitionTime="0.30000001" from="all_co_sk_shout_anim" to="Skill End"/>
        <Link transitionTime="0" from="Skill Start" to="twoswords_co_sk_weapon_slash_3"/>
        <Link transitionTime="0" from="twoswords_co_sk_weapon_slash_3" to="twoswords_co_sk_weapon_slash_3_anim"/>
        <Link transitionTime="0" from="twoswords_co_sk_weapon_slash_3_anim" to="Skill End"/>
        <Link transitionTime="0" from="Skill End" to="Main_Hub"/>
      </Links>
    </AnimationGraph>"#;

    fn templates(name: &str) -> Option<String> {
        match name {
            "IdleOneShotNoLookIk" => Some(r#"<AGTemplate name="IdleOneShotNoLookIk" extend="IdleOneShot"><LookIk override="1"/></AGTemplate>"#.into()),
            "IdleOneShot" => Some(r#"<AGTemplate name="IdleOneShot"><TransitionParamsLayer1 TransitionTime="iff( $transition_time == 0, 0.2, $transition_time )"/></AGTemplate>"#.into()),
            _ => None,
        }
    }

    /// The blends measured on the 1.2 client follow from the links (skill parity trace, 2026-11-03).
    #[test]
    fn blends_are_the_largest_link_walked_else_the_states_own() {
        let g = ActionGraph::parse(GRAPH, templates);
        let near = |a: Option<f32>, b: f32| a.is_some_and(|a| (a - b).abs() < 1e-4);
        // the first clip of Triple Slash, then the second after the first
        assert!(near(g.action_blend(0.0, "twohand_co_sk_streakattack_1"), 0.01));
        assert!(near(g.action_blend(g.exit("twohand_co_sk_streakattack_1"), "twohand_co_sk_streakattack_1"), 0.2));
        assert!(near(g.action_blend(0.0, "all_co_sk_shout"), 0.2));
        // Whirlwind Slash: no link time, the state's own
        assert!(near(g.action_blend(0.0, "twoswords_co_sk_weapon_slash_3"), 0.1));
        // the idle after a shout, after Whirlwind Slash
        assert!((ActionGraph::idle_blend(g.exit("all_co_sk_shout")) - 0.3).abs() < 1e-4);
        assert!((ActionGraph::idle_blend(g.exit("twoswords_co_sk_weapon_slash_3")) - DEFAULT_BLEND).abs() < 1e-4);
        // the template default through `extend`
        assert!(near(g.own("twohand_co_sk_streakattack_1"), 0.2));
        assert_eq!(g.action_blend(0.0, "wolf_ba_idle"), None, "not a state of the graph");
    }

    #[test]
    fn template_defaults_read_numbers_and_iff_expressions() {
        assert_eq!(transition_default("0.1"), Some(0.1));
        assert_eq!(transition_default("iff( $transition_time == 0, 0.2, $transition_time )"), Some(0.2));
        assert_eq!(transition_default("$transition_time"), None);
    }

    /// The game's graph, when the pak is there.
    #[test]
    fn the_games_graph_gives_the_measured_blends() {
        if !aa_cry::pak_available() {
            return;
        }
        let g = ActionGraph::get();
        assert!(g.knows("twohand_co_sk_streakattack_2"), "the pak's graph was read");
        let near = |a: Option<f32>, b: f32| a.is_some_and(|a| (a - b).abs() < 1e-3);
        assert!(near(g.action_blend(0.0, "twohand_co_sk_streakattack_1"), 0.01));
        assert!(near(g.action_blend(g.exit("twohand_co_sk_streakattack_1"), "twohand_co_sk_streakattack_2"), 0.2));
        assert!(near(g.action_blend(0.0, "twoswords_co_sk_weapon_slash_3"), 0.1));
        assert!(near(g.action_blend(0.0, "all_co_sk_shout"), 0.2));
        assert!(near(g.action_blend(0.0, "twohand_co_attack"), 0.2), "Combat Start -> co_attack");
        assert!(near(g.action_blend(g.exit("twohand_co_sk_weapon_blunt_3_cast"), "twohand_co_sk_weapon_blunt_3_launch"), 0.2));
        assert!((ActionGraph::idle_blend(g.exit("all_co_sk_shout")) - 0.3).abs() < 1e-3);
        assert!((g.exit("twohand_co_sk_streakattack_1") - 0.2).abs() < 1e-3, "no hub inflates an exit");
        assert!((g.exit("twohand_co_attack") - 0.2).abs() < 0.11, "{}", g.exit("twohand_co_attack"));
    }
}
