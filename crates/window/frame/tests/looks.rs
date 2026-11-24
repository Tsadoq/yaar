//! GPUI's test platform reports a light system and cannot change it, so the test tells `Looks` the
//! system itself.

mod common;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{
    AppContext, Context, InteractiveElement, IntoElement, Render, Size, Styled, TestAppContext,
    VisualTestContext, Window, div, px, size,
};
use yaar_frame::theme::{Appearance, Colors, DARK, LIGHT, applied, dark, light};
use yaar_frame::{BuildPane, KeeperLink, Models, Pane};
use yaar_link::{Fact, Setting, ThemeChoice, TurnEndSelects};

use common::{Shown, open_frame, open_frame_staged, wait_until};

const WINDOW: Size<gpui_kit::Pixels> = size(px(1280.), px(800.));

fn named(name: &str) -> Setting {
    Setting::Theme(ThemeChoice::Named(name.to_owned()))
}

fn opened(cx: &mut TestAppContext) -> (Shown, VisualTestContext) {
    open_frame(WINDOW, Vec::new(), cx)
}

fn kit(cx: &mut VisualTestContext) -> (ThemeMode, Colors) {
    cx.update(|_, app| {
        let theme = app.global::<Theme>();
        (theme.mode, Colors::of(&theme.colors))
    })
}

fn background(cx: &mut VisualTestContext) -> gpui_kit::Hsla {
    cx.update(|_, app| app.global::<Theme>().colors.background)
}

fn applied_name(cx: &mut VisualTestContext) -> String {
    cx.update(|_, app| applied(app).name)
}

fn system_is(shown: &Shown, appearance: Appearance, cx: &mut VisualTestContext) {
    shown
        .models
        .looks
        .update(cx, |looks, cx| looks.system_is(appearance, cx));
    cx.run_until_parked();
}

fn told(shown: &mut Shown, setting: Setting, cx: &mut VisualTestContext) {
    let Setting::Theme(choice) = setting.clone() else {
        panic!("a test of the theme tells a theme");
    };
    let mut kept = shown
        .models
        .settings
        .read_with(cx, |list, _| list.all().to_vec());
    kept.retain(|held| !held.same_thing_as(&setting));
    kept.push(setting.clone());
    shown.keeper.set_settings(kept);
    shown.keeper.tell(Fact::SettingChanged(setting));
    wait_until(cx, "the choice of theme to reach the model", |cx| {
        shown
            .models
            .settings
            .read_with(cx, |list, _| list.theme() == choice)
    });
    cx.run_until_parked();
}

#[track_caller]
fn shows(
    mode: ThemeMode,
    name: &str,
    definition: &yaar_frame::theme::ThemeDefinition,
    cx: &mut VisualTestContext,
) {
    let (kit_mode, colors) = kit(cx);
    assert_eq!(kit_mode, mode, "the toolkit's mode");
    assert_eq!(colors, definition.colors, "the toolkit's colours");
    assert_eq!(applied_name(cx), name, "the name `applied` gives");
    assert_eq!(
        cx.update(|_, app| applied(app)),
        *definition,
        "the definition `applied` gives, terminal colours included"
    );
}

type Found = (Option<Appearance>, ThemeMode, Colors);

struct Empty {
    focus: gpui_kit::FocusHandle,
}

impl Render for Empty {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full()
    }
}

/// A pane that writes down what the model and the toolkit hold as it is built: panes are built
/// inside the toolkit's opening of the window, so that is what the first frame is drawn in.
fn finding(found: &std::rc::Rc<std::cell::RefCell<Option<Found>>>) -> BuildPane {
    let found = found.clone();
    Box::new(move |models, _, cx| {
        let system = models.looks.read(cx).system();
        let theme = cx.global::<Theme>();
        *found.borrow_mut() = Some((system, theme.mode, Colors::of(&theme.colors)));
        let focus = cx.focus_handle();
        let view = cx.new(|_| Empty {
            focus: focus.clone(),
        });
        Pane {
            name: "finding",
            view: view.into(),
            focus,
        }
    })
}

#[gpui_kit::test]
fn a_window_made_by_open_is_in_yaars_definition_of_the_applications_appearance_with_nobody_telling_it(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    // The toolkit is put in colours no definition holds: what the first frame would be drawn in
    // were nothing applied before the window opens.
    cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    let found = std::rc::Rc::new(std::cell::RefCell::new(None));
    let opened = cx.update(|cx| {
        yaar_frame::open(
            std::path::PathBuf::from("/nonexistent/yaar/keeper.sock"),
            "9.9.9",
            vec![finding(&found)],
            cx,
        )
        .expect("open the window")
    });

    let (system, mode, colors) = found
        .borrow_mut()
        .take()
        .expect("the pane was built while the window was opened");
    assert_eq!(
        system,
        Some(Appearance::Light),
        "the system the model holds when the panes are built"
    );
    assert_eq!(
        mode,
        ThemeMode::Light,
        "the toolkit's mode when the panes are built"
    );
    assert_eq!(
        colors,
        light().colors,
        "the toolkit's colours when the panes are built"
    );

    let (system, definition) = opened
        .models
        .looks
        .read_with(cx, |looks, _| (looks.system(), looks.applied().clone()));
    assert_eq!(
        system,
        Some(Appearance::Light),
        "the system the model holds"
    );
    assert_eq!(definition, light());
    let (mode, colors, held) = cx.update(|cx| {
        let theme = cx.global::<Theme>();
        (theme.mode, Colors::of(&theme.colors), applied(cx))
    });
    assert_eq!(mode, ThemeMode::Light, "the toolkit's mode");
    assert_eq!(colors, light().colors, "the toolkit's colours");
    assert_eq!(held, light(), "the definition `applied` gives");
}

#[gpui_kit::test]
fn nothing_is_applied_and_no_system_is_known_before_the_window_tells_it(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    let before = cx.update(|cx| {
        let theme = cx.global::<Theme>();
        (theme.mode, Colors::of(&theme.colors))
    });

    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link, cx));
    models.settings.update(cx, |list, cx| {
        list.set(Setting::Theme(ThemeChoice::Named(LIGHT.to_owned())), cx);
    });
    cx.run_until_parked();

    models.looks.read_with(cx, |looks, cx| {
        assert_eq!(looks.system(), None);
        assert_eq!(looks.chosen(cx), ThemeChoice::Named(LIGHT.to_owned()));
    });
    let after = cx.update(|cx| {
        let theme = cx.global::<Theme>();
        (theme.mode, Colors::of(&theme.colors))
    });
    assert_eq!(after, before, "the toolkit's theme is as it was");
    assert_eq!(
        cx.update(|cx| applied(cx)),
        light(),
        "and `applied` holds the light one"
    );

    models
        .looks
        .update(cx, |looks, cx| looks.system_is(Appearance::Dark, cx));
    cx.update(|cx| assert_eq!(applied(cx).name, LIGHT));
    assert_eq!(
        cx.update(|cx| cx.global::<Theme>().mode),
        ThemeMode::Light,
        "light, though the system is dark and the kit was dark"
    );
}

#[gpui_kit::test]
fn with_no_theme_chosen_the_app_follows_the_system_as_it_changes(cx: &mut TestAppContext) {
    let (shown, mut cx) = opened(cx);
    shown.models.looks.read_with(&cx, |looks, cx| {
        assert_eq!(looks.system(), None);
        assert_eq!(looks.chosen(cx), ThemeChoice::System);
    });

    system_is(&shown, Appearance::Light, &mut cx);
    shown.models.looks.read_with(&cx, |looks, _| {
        assert_eq!(looks.system(), Some(Appearance::Light));
        assert_eq!(looks.applied().name, LIGHT);
    });
    shows(ThemeMode::Light, LIGHT, &light(), &mut cx);

    system_is(&shown, Appearance::Dark, &mut cx);
    shown.models.looks.read_with(&cx, |looks, _| {
        assert_eq!(looks.system(), Some(Appearance::Dark));
        assert_eq!(looks.applied().name, DARK);
    });
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
    let (_, colors) = kit(&mut cx);
    assert_eq!(colors.background, dark().colors.background);

    system_is(&shown, Appearance::Light, &mut cx);
    shows(ThemeMode::Light, LIGHT, &light(), &mut cx);
}

#[gpui_kit::test]
fn the_definition_s_own_colours_are_in_the_toolkit_over_the_mode_s_own(cx: &mut TestAppContext) {
    let (shown, mut cx) = opened(cx);
    // The definition differs from the toolkit's own dark theme somewhere; otherwise this shows
    // nothing of the second update.
    cx.update(|_, app| Theme::change(ThemeMode::Dark, None, app));
    let (_, kits_own) = kit(&mut cx);
    assert_ne!(
        kits_own,
        dark().colors,
        "the dark definition holds a colour of its own"
    );

    system_is(&shown, Appearance::Dark, &mut cx);
    let (_, colors) = kit(&mut cx);
    assert_eq!(colors, dark().colors);
}

#[gpui_kit::test]
fn after_a_choice_the_system_is_not_followed(cx: &mut TestAppContext) {
    let (mut shown, mut cx) = opened(cx);
    system_is(&shown, Appearance::Light, &mut cx);
    shows(ThemeMode::Light, LIGHT, &light(), &mut cx);

    told(&mut shown, named(DARK), &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
    shown.models.looks.read_with(&cx, |looks, cx| {
        assert_eq!(looks.chosen(cx), ThemeChoice::Named(DARK.to_owned()));
        assert_eq!(looks.applied().name, DARK);
    });

    system_is(&shown, Appearance::Light, &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
    system_is(&shown, Appearance::Dark, &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
    system_is(&shown, Appearance::Light, &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
    shown.models.looks.read_with(&cx, |looks, _| {
        assert_eq!(looks.system(), Some(Appearance::Light))
    });

    told(&mut shown, named(LIGHT), &mut cx);
    shows(ThemeMode::Light, LIGHT, &light(), &mut cx);
    system_is(&shown, Appearance::Dark, &mut cx);
    shows(ThemeMode::Light, LIGHT, &light(), &mut cx);
}

#[gpui_kit::test]
fn a_choice_of_the_system_follows_it_again_at_once(cx: &mut TestAppContext) {
    let (mut shown, mut cx) = opened(cx);
    system_is(&shown, Appearance::Light, &mut cx);
    told(&mut shown, named(DARK), &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);

    told(&mut shown, Setting::Theme(ThemeChoice::System), &mut cx);
    shows(ThemeMode::Light, LIGHT, &light(), &mut cx);

    system_is(&shown, Appearance::Dark, &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
    system_is(&shown, Appearance::Light, &mut cx);
    shows(ThemeMode::Light, LIGHT, &light(), &mut cx);
}

#[gpui_kit::test]
fn a_name_no_theme_has_is_the_system_s_setting(cx: &mut TestAppContext) {
    let (mut shown, mut cx) = opened(cx);
    system_is(&shown, Appearance::Dark, &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);

    told(&mut shown, named("Sepia"), &mut cx);
    shown.models.looks.read_with(&cx, |looks, cx| {
        assert_eq!(looks.chosen(cx), ThemeChoice::Named("Sepia".to_owned()));
        assert!(looks.themes().named("Sepia").is_none());
    });
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);

    system_is(&shown, Appearance::Light, &mut cx);
    shows(ThemeMode::Light, LIGHT, &light(), &mut cx);
    system_is(&shown, Appearance::Dark, &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
}

#[gpui_kit::test]
fn a_choice_the_keeper_held_at_the_connection_is_applied_at_the_first_telling(
    cx: &mut TestAppContext,
) {
    let (shown, mut cx) = open_frame_staged(hello(), WINDOW, Vec::new(), cx, |keeper| {
        keeper.set_settings(vec![named(DARK)])
    });
    wait_until(&mut cx, "the settings to reach the model", |cx| {
        shown.models.settings.read_with(cx, |list, _| {
            list.theme() == ThemeChoice::Named(DARK.to_owned())
        })
    });
    shown
        .models
        .looks
        .read_with(&cx, |looks, _| assert_eq!(looks.system(), None));
    assert_ne!(applied_name(&mut cx), DARK);

    system_is(&shown, Appearance::Light, &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
}

#[gpui_kit::test]
fn a_choice_the_window_makes_is_in_force_before_the_keeper_answers(cx: &mut TestAppContext) {
    let (shown, mut cx) = opened(cx);
    system_is(&shown, Appearance::Light, &mut cx);

    shown.models.settings.update(&mut cx, |list, cx| {
        list.set(named(DARK), cx);
    });
    cx.run_until_parked();
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);

    shown.models.settings.update(&mut cx, |list, cx| {
        list.set(Setting::Theme(ThemeChoice::System), cx);
    });
    cx.run_until_parked();
    shows(ThemeMode::Light, LIGHT, &light(), &mut cx);
}

#[gpui_kit::test]
fn what_is_applied_already_is_not_applied_again(cx: &mut TestAppContext) {
    let (mut shown, mut cx) = opened(cx);
    system_is(&shown, Appearance::Light, &mut cx);

    // Colours changed by hand: applying again would write the definition over them.
    let marker = gpui_kit::hsla(0.5, 0.5, 0.5, 1.0);
    cx.update(|_, app| {
        Theme::update(app, |theme| theme.colors.background = marker);
    });
    cx.run_until_parked();

    system_is(&shown, Appearance::Light, &mut cx);
    assert_eq!(background(&mut cx), marker);

    shown
        .keeper
        .set_settings(vec![Setting::TurnEnd(TurnEndSelects::KeepSelection)]);
    shown.keeper.tell(Fact::SettingChanged(Setting::TurnEnd(
        TurnEndSelects::KeepSelection,
    )));
    wait_until(&mut cx, "the other setting to reach the model", |cx| {
        shown.models.settings.read_with(cx, |list, _| {
            list.all()
                .contains(&Setting::TurnEnd(TurnEndSelects::KeepSelection))
        })
    });
    cx.run_until_parked();
    assert_eq!(background(&mut cx), marker);

    told(&mut shown, named(LIGHT), &mut cx);
    assert_eq!(
        background(&mut cx),
        marker,
        "light chosen while light is applied"
    );

    system_is(&shown, Appearance::Dark, &mut cx);
    told(&mut shown, Setting::Theme(ThemeChoice::System), &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
}

#[gpui_kit::test]
fn a_named_theme_given_back_to_the_system_while_the_definition_stays_runs_the_last_step_only(
    cx: &mut TestAppContext,
) {
    use std::cell::Cell;
    use std::rc::Rc;

    let (mut shown, mut cx) = opened(cx);
    system_is(&shown, Appearance::Light, &mut cx);
    told(&mut shown, named(LIGHT), &mut cx);
    let marker = gpui_kit::hsla(0.5, 0.5, 0.5, 1.0);
    cx.update(|_, app| {
        Theme::update(app, |theme| theme.colors.background = marker);
    });
    cx.run_until_parked();
    let applied_before = cx.update(|_, app| applied(app));

    let told_to = Rc::new(Cell::new(0));
    let _watch = cx.update({
        let looks = shown.models.looks.clone();
        let told_to = told_to.clone();
        move |_, app| app.observe(&looks, move |_, _| told_to.set(told_to.get() + 1))
    });
    told(&mut shown, Setting::Theme(ThemeChoice::System), &mut cx);

    assert_eq!(
        told_to.get(),
        1,
        "the change of the forced appearance is told once"
    );
    assert_eq!(background(&mut cx), marker, "no step wrote the definition");
    assert_eq!(
        cx.update(|_, app| applied(app)),
        applied_before,
        "`applied` is as it was"
    );
    shown.models.looks.read_with(&cx, |looks, cx| {
        assert_eq!(looks.chosen(cx), ThemeChoice::System);
        assert_eq!(looks.applied().name, LIGHT);
    });
}

fn hello() -> yaar_link::Hello {
    yaar_link::Hello {
        protocol: yaar_link::PROTOCOL_VERSION,
        pid: 1,
        app_version: "9.9.9".to_owned(),
    }
}

#[gpui_kit::test]
fn the_themes_are_the_built_in_ones_and_the_model_tells_its_observers_of_a_change(
    cx: &mut TestAppContext,
) {
    use std::cell::Cell;
    use std::rc::Rc;

    let (shown, mut cx) = opened(cx);
    shown.models.looks.read_with(&cx, |looks, _| {
        assert_eq!(looks.themes(), &yaar_frame::theme::Themes::built_in());
    });
    let told = Rc::new(Cell::new(0));
    let _watch = cx.update({
        let looks = shown.models.looks.clone();
        let told = told.clone();
        move |_, app| app.observe(&looks, move |_, _| told.set(told.get() + 1))
    });

    system_is(&shown, Appearance::Dark, &mut cx);
    assert_eq!(told.get(), 1, "a change is told once");
    system_is(&shown, Appearance::Dark, &mut cx);
    assert_eq!(told.get(), 1, "the same again is told to nobody");
    system_is(&shown, Appearance::Light, &mut cx);
    assert_eq!(told.get(), 2);
}
