//! Component test (FRAME-14): the theme the Window draws in, through the model the Frame
//! shows it by ([`yaar_frame::Looks`]) and the toolkit's own theme beneath it.
//!
//! GPUI's test platform reports a light system and gives a test no way to change it, so the
//! test tells the model what the platform would, at [`Looks::system_is`], as the Window
//! program does. The user's choice comes the way the Keeper's settings reach a Window: a
//! `Fact::SettingChanged` told by the test's Keeper, and `SettingList::set` for a choice
//! the Window makes itself. Every read is of the toolkit's theme, which every control
//! draws with, and of what `yaar_frame::theme::applied` gives the terminal pane.
//!
//! Every telling, and every choice the test makes, is followed by a barrier
//! (`run_until_parked`) before anything is read: a negative clause ("it does not follow",
//! "nothing is rewritten", "told to nobody") would otherwise pass on a model that follows
//! one task late. The settings reaching the model are waited for with the deadline.

mod common;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{AppContext, Size, TestAppContext, VisualTestContext, px, size};
use yaar_frame::theme::{Appearance, Colors, DARK, LIGHT, applied, dark, light};
use yaar_frame::{KeeperLink, Models};
use yaar_link::{Fact, Setting, ThemeChoice, TurnEndSelects};

use common::{Shown, open_frame, open_frame_staged, wait_until};

const WINDOW: Size<gpui_kit::Pixels> = size(px(1280.), px(800.));

fn named(name: &str) -> Setting {
    Setting::Theme(ThemeChoice::Named(name.to_owned()))
}

/// A Frame opened with nothing set, and the kit prepared, as the Window program leaves it.
fn opened(cx: &mut TestAppContext) -> (Shown, VisualTestContext) {
    open_frame(WINDOW, Vec::new(), cx)
}

/// What the toolkit draws with: its mode and the colour of every role.
fn kit(cx: &mut VisualTestContext) -> (ThemeMode, Colors) {
    cx.update(|_, app| {
        let theme = app.global::<Theme>();
        (theme.mode, Colors::of(&theme.colors))
    })
}

/// The toolkit's ground colour alone.
fn background(cx: &mut VisualTestContext) -> gpui_kit::Hsla {
    cx.update(|_, app| app.global::<Theme>().colors.background)
}

fn applied_name(cx: &mut VisualTestContext) -> String {
    cx.update(|_, app| applied(app).name)
}

/// Tells the Looks what the platform would say of the system, and lets everything the
/// telling set off run, so that a read after it sees what the model came to, late or not.
fn system_is(shown: &Shown, appearance: Appearance, cx: &mut VisualTestContext) {
    shown
        .models
        .looks
        .update(cx, |looks, cx| looks.system_is(appearance, cx));
    cx.run_until_parked();
}

/// The Keeper tells every Window that the theme is this; waits for the model of the
/// settings to hold it, which is when `Looks` has seen it too.
fn told(shown: &mut Shown, setting: Setting, cx: &mut VisualTestContext) {
    let Setting::Theme(choice) = setting.clone() else {
        panic!("a test of the theme tells a theme");
    };
    // The Keeper keeps what it tells, as the real one does: the setting takes the place of
    // the one it is the same thing as and the others stay, for the window may still ask for
    // every setting after hearing the fact, and the answer must agree with it.
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
    // What the choice set off has run before anything is read.
    cx.run_until_parked();
}

/// Whether the toolkit and the terminal pane's global show this definition: its mode, every
/// colour it has for a role, and its name.
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

#[gpui_kit::test]
fn nothing_is_applied_and_no_system_is_known_before_the_window_tells_it(cx: &mut TestAppContext) {
    // The kit is in a mode the definitions would not leave it in if `Looks` applied one:
    // dark, where the light one is what `applied` holds before anything was applied.
    cx.update(gpui_kit::init);
    cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    let before = cx.update(|cx| {
        let theme = cx.global::<Theme>();
        (theme.mode, Colors::of(&theme.colors))
    });

    let link = cx.new(|_| KeeperLink::new());
    let models = cx.update(|cx| Models::new(link, cx));
    // A choice is made too, which would be applied if anything were.
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

    // The first telling applies the choice, the system's setting not being what decides.
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
    // What the toolkit has of its own for the dark mode, and that the definition differs
    // from it somewhere: otherwise this test would show nothing of the second update.
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

    // The Keeper tells that dark is chosen: dark at once, the system being light.
    told(&mut shown, named(DARK), &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
    shown.models.looks.read_with(&cx, |looks, cx| {
        assert_eq!(looks.chosen(cx), ThemeChoice::Named(DARK.to_owned()));
        assert_eq!(looks.applied().name, DARK);
    });

    // The system changes, and is noted, and changes nothing.
    system_is(&shown, Appearance::Light, &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
    system_is(&shown, Appearance::Dark, &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
    system_is(&shown, Appearance::Light, &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
    shown.models.looks.read_with(&cx, |looks, _| {
        assert_eq!(looks.system(), Some(Appearance::Light))
    });

    // And the other way: light chosen while the system is dark.
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

    // As the system: light at once, the system being light as it was last told.
    told(&mut shown, Setting::Theme(ThemeChoice::System), &mut cx);
    shows(ThemeMode::Light, LIGHT, &light(), &mut cx);

    // And it follows again.
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

    // A theme of an older app, or of a newer one, that this app has no definition of.
    told(&mut shown, named("Sepia"), &mut cx);
    shown.models.looks.read_with(&cx, |looks, cx| {
        assert_eq!(looks.chosen(cx), ThemeChoice::Named("Sepia".to_owned()));
        assert!(looks.themes().named("Sepia").is_none());
    });
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);

    // It is the system's setting that decides, so it is followed.
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
    // Holding the choice applies nothing: no system's setting is known.
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

    // The toolkit's colours are changed by hand; applying anything again would write the
    // definition's over them, which is what this test looks for.
    let marker = gpui_kit::hsla(0.5, 0.5, 0.5, 1.0);
    cx.update(|_, app| {
        Theme::update(app, |theme| theme.colors.background = marker);
    });
    cx.run_until_parked();

    // The system's setting told again as it was.
    system_is(&shown, Appearance::Light, &mut cx);
    assert_eq!(background(&mut cx), marker);

    // A setting that is not the theme changes, and the settings model tells its observers.
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

    // The theme chosen as it already is in force: the system's light one, by its name.
    told(&mut shown, named(LIGHT), &mut cx);
    assert_eq!(
        background(&mut cx),
        marker,
        "light chosen while light is applied"
    );

    // A change of the definition does write it.
    system_is(&shown, Appearance::Dark, &mut cx);
    told(&mut shown, Setting::Theme(ThemeChoice::System), &mut cx);
    shows(ThemeMode::Dark, DARK, &dark(), &mut cx);
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
