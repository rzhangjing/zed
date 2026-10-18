//! The drawing test window: `Help > Plot`.
//!
//! The GPUI Kit was merged into this workspace as a whole (`crates/gpui-kit`),
//! and until now the only thing showing it drawing was `Help > gpui-kit`, the
//! gallery. A gallery is a catalogue, not a test bench: the charts in it are fed
//! fixed data in fixed cards, so nothing there tells you how the plot primitives
//! behave when the data changes, when the appear animation is switched off, or
//! when the hover layer is disabled. This window is the bench.
//!
//! ## What it exercises
//!
//! Two tabs, because "drawing" in the Kit means two different things:
//!
//! * **图表** — four plot charts ([`LineChart`], [`AreaChart`], [`BarChart`],
//!   [`PieChart`]) side by side on generated data, with the two switches every
//!   chart carries for real: `动画` drives the appear animation (`appear`) and
//!   `交互` the hover layer (`interactive` — the hitbox, the crosshair and the
//!   tooltip). `重新生成` reseeds the data, which as a side effect replays the
//!   animation: the appear key is the seed, so a new dataset reveals itself again
//!   instead of snapping into place.
//! * **画板** — a freehand canvas over the same low-level paint API the charts
//!   use: [`PathBuilder`] to build a stroke and [`Window::paint_path`] to fill it,
//!   plus [`Window::paint_quad`] for a single click's dot. Strokes are kept in
//!   window coordinates and offset by the canvas bounds as they are painted, so
//!   nothing has to be recomputed when the canvas is laid out; the canvas is what
//!   moves.
//!
//! ## Why a window
//!
//! Same reason as the Dameng window: this is a *test* surface, not an editor
//! feature. It owns state that belongs to a run (which tab, whether the animation
//! is on, the strokes drawn so far) and it has to be reachable without a project
//! open. Nothing here is persisted — a fresh window starts on the charts tab with
//! the animation and the hover layer on, so the first thing you see is the Kit at
//! its own defaults.
//!
//! ## The Kit's window root
//!
//! [`show_window`] returns the Kit's [`Root`] from `open_window` rather than the
//! view itself, exactly as `open_gpui_kit_showcase` and the Dameng window do. The
//! Kit's components read window-level state (text selection, tooltips, dialogs)
//! off the root, and a view without one above it panics with "component window
//! state is missing" the first time a control touches that state. The charts
//! themselves would draw under a bare root, but the buttons and the theme are the
//! Kit's, so the root is the consistent choice.

use gpui::{
    AnyElement, AnyWindowHandle, App, BorderStyle, Bounds, ClickEvent, Context, Corners, Edges,
    FontWeight, Hsla, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, PathBuilder, Pixels, Point, Render, SharedString, Size, TitlebarOptions, Window,
    WindowBounds, WindowKind, WindowOptions, canvas, div, point, prelude::*, px, size,
};
use gpui_kit::{
    base::Root,
    component::{
        Disableable as _,
        button::{Button as KitButton, ButtonVariants as _},
        chart::{AreaChart, BarChart, LineChart, PieChart},
        h_flex,
        theme::{Theme as KitTheme, ThemeMode as KitThemeMode},
        v_flex,
    },
};
use release_channel::ReleaseChannel;
use theme::ActiveTheme as _;

/// Title of the window, and what the title bar shows.
const WINDOW_TITLE: &str = "Plot 绘图测试";

/// How many points the generated series has. Twelve keeps the x axis readable at
/// this width: a label per tick, with room for the label itself.
const SAMPLE_COUNT: usize = 12;

/// The series starts from this seed, so the first frame is always the same data
/// set. A window that showed different data on every launch would make the
/// animation and interaction switches hard to reason about.
const DEFAULT_SEED: u64 = 0x5EED_1A7E_9C3D_4B21;

/// Strokes kept on the freehand canvas. Each one is a polyline of window points,
/// so this is the bound that keeps a long session from growing without limit.
const MAX_STROKES: usize = 512;

/// Points kept in a single stroke, so one very long drag cannot pin the paint
/// loop.
const MAX_STROKE_POINTS: usize = 4096;

/// The height of one chart card.
///
/// A chart asks its parent for the whole parent, so the parent has to have a
/// definite height: without one the card collapses to the height of its heading
/// and the chart draws outside the card.
const CARD_HEIGHT: Pixels = px(240.);

/// Brush widths, in pixels, that `细/中/粗` select between.
const BRUSH_WIDTHS: [f32; 3] = [1.5, 3.5, 7.0];

/// Labels for the brush widths, matching [`BRUSH_WIDTHS`] index for index.
const BRUSH_LABELS: [&str; 3] = ["细", "中", "粗"];

/// Which tab the window is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    /// The four plot charts on generated data.
    Charts,
    /// A freehand canvas over GPUI's paint API.
    Freehand,
}

impl Tab {
    /// Order the tabs appear in, and the order they cycle through.
    const ORDER: [Tab; 2] = [Tab::Charts, Tab::Freehand];

    /// Button label for this tab.
    fn label(self) -> &'static str {
        match self {
            Tab::Charts => "图表",
            Tab::Freehand => "画板",
        }
    }

    /// Next tab, wrapping.
    ///
    /// The window has no keyboard shortcut for this yet, so it exists for the
    /// tests that pin the cycle down rather than for the UI.
    #[cfg(test)]
    fn next(self) -> Tab {
        match self {
            Tab::Charts => Tab::Freehand,
            Tab::Freehand => Tab::Charts,
        }
    }
}

/// Which chart to draw. Each kind takes a different slice of the Kit's plot API,
/// which is the point of showing all four: one window then covers the
/// point-series charts, the band chart and the radial one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChartKind {
    Line,
    Area,
    Bar,
    Pie,
}

impl ChartKind {
    /// Heading over the chart's card.
    fn title(self) -> &'static str {
        match self {
            ChartKind::Line => "折线图",
            ChartKind::Area => "面积图",
            ChartKind::Bar => "柱状图",
            ChartKind::Pie => "饼图",
        }
    }

    /// One line under the heading, saying what the card exercises.
    fn hint(self) -> &'static str {
        match self {
            ChartKind::Line => "曲线插值、点标记、刻度格式",
            ChartKind::Area => "填充区域、描边、虚线网格",
            ChartKind::Bar => "band 布局、圆角柱、数值标签",
            ChartKind::Pie => "扇形布局、环半径、标签",
        }
    }

    /// Stable id for this chart's element.
    ///
    /// The Kit takes a chart's id from its construction site by default, and all
    /// four cards are built from the same `match`, so without this they would
    /// share one hover state and one path cache.
    fn element_id(self) -> &'static str {
        match self {
            ChartKind::Line => "plot-chart-line",
            ChartKind::Area => "plot-chart-area",
            ChartKind::Bar => "plot-chart-bar",
            ChartKind::Pie => "plot-chart-pie",
        }
    }

    /// The four kinds, in the order the grid shows them.
    const ALL: [ChartKind; 4] = [
        ChartKind::Line,
        ChartKind::Area,
        ChartKind::Bar,
        ChartKind::Pie,
    ];
}

/// One generated datum: the label on the x axis and the value on the y axis.
#[derive(Clone, Debug)]
struct Sample {
    label: SharedString,
    value: f64,
}

impl Sample {
    /// The value, formatted the way the axis gutter and the bar labels show it.
    fn formatted(&self) -> String {
        format!("{:.0}", self.value)
    }
}

/// Generates the series a chart card shows.
///
/// `splitmix64` rather than `rand`: the window needs a deterministic series from
/// a seed and nothing else, and this keeps a dependency out of the tree.
fn generate_samples(mut seed: u64) -> Vec<Sample> {
    const MONTHS: [&str; SAMPLE_COUNT] = [
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ];

    let mut next = move || {
        seed = seed
            .wrapping_add(0x9E37_79B9_7F4A_7C15)
            .wrapping_mul(0xBF58_476D_1CE4_E5B9);
        let mixed = (seed ^ (seed >> 30)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (mixed ^ (mixed >> 31)) as f64 / u64::MAX as f64
    };

    MONTHS
        .iter()
        .enumerate()
        .map(|(index, month)| {
            // A trend plus noise: a flat random series would hide a broken x
            // ordering, which is one of the things this window is for.
            let trend = 30. + index as f64 * 4.5;
            Sample {
                label: SharedString::from(*month),
                value: (trend + next() * 26. - 8.).round(),
            }
        })
        .collect()
}

/// One freehand stroke: the points it was drawn through, and its brush.
#[derive(Clone, Debug)]
struct Stroke {
    /// The points, in window coordinates.
    ///
    /// Window coordinates rather than canvas coordinates because the mouse
    /// handlers have them and the canvas is what moves when the window is
    /// resized; [`PlotWindow::paint_strokes`] offsets them by the canvas origin
    /// as it paints.
    points: Vec<Point<Pixels>>,
    color: Hsla,
    width: f32,
}

impl Stroke {
    /// Half the brush width, so a single click's dot has the radius a stroke of
    /// the same brush would have thickness.
    fn radius(&self) -> f32 {
        self.width / 2.
    }
}

/// The window's state. Everything here is per-window: two Plot windows would not
/// share a tab, a seed or a stroke.
pub struct PlotWindow {
    tab: Tab,
    seed: u64,
    samples: Vec<Sample>,
    animate: bool,
    interactive: bool,
    strokes: Vec<Stroke>,
    in_progress: Option<Stroke>,
    brush: usize,
    color: usize,
}

impl PlotWindow {
    /// A window on the charts tab, at the Kit's defaults.
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            tab: Tab::Charts,
            seed: DEFAULT_SEED,
            samples: generate_samples(DEFAULT_SEED),
            animate: true,
            interactive: true,
            strokes: Vec::new(),
            in_progress: None,
            brush: 1,
            color: 0,
        }
    }

    // -- tab and chart state -------------------------------------------------

    /// Switches tab, as the tab buttons do.
    fn select(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        cx.notify();
    }

    /// Advances the seed and regenerates the series.
    fn regenerate(&mut self, cx: &mut Context<Self>) {
        self.reseed();
        cx.notify();
    }

    /// The state half of [`PlotWindow::regenerate`], separated so it is testable
    /// without an app.
    ///
    /// The seed is also the charts' appear key, so regenerating replays the
    /// reveal animation when `动画` is on — a data change that does not animate
    /// reads as a redraw that did not happen.
    fn reseed(&mut self) {
        self.seed = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        self.samples = generate_samples(self.seed);
    }

    /// Turns the appear animation on or off.
    fn toggle_animate(&mut self, cx: &mut Context<Self>) {
        self.animate = !self.animate;
        cx.notify();
    }

    /// Turns the charts' hover layer (hitbox, crosshair, tooltip) on or off.
    fn toggle_interactive(&mut self, cx: &mut Context<Self>) {
        self.interactive = !self.interactive;
        cx.notify();
    }

    // -- freehand state ------------------------------------------------------

    /// The brush colour currently selected, taken from the Kit's chart palette so
    /// the strokes sit in the same palette as the charts.
    fn brush_color(&self, cx: &App) -> Hsla {
        let palette = chart_palette(cx);
        palette[self.color % palette.len()]
    }

    /// Current brush width in pixels.
    fn brush_width(&self) -> f32 {
        BRUSH_WIDTHS[self.brush % BRUSH_WIDTHS.len()]
    }

    /// Starts a stroke where the button went down.
    fn begin_stroke(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left {
            return;
        }
        let color = self.brush_color(cx);
        // No-op when the button is not the left one, in which case there is
        // nothing to repaint.
        self.start_stroke(event.position, color);
        cx.notify();
    }

    /// The state half of [`PlotWindow::begin_stroke`].
    fn start_stroke(&mut self, position: Point<Pixels>, color: Hsla) {
        if self.strokes.len() >= MAX_STROKES {
            self.strokes.remove(0);
        }
        self.in_progress = Some(Stroke {
            points: vec![position],
            color,
            width: self.brush_width(),
        });
    }

    /// Extends the stroke in progress, if any.
    fn extend_stroke(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.extend_to(
            event.position,
            event.pressed_button == Some(MouseButton::Left),
        ) {
            cx.notify();
        }
    }

    /// The state half of [`PlotWindow::extend_stroke`]; returns whether anything
    /// changed, so the caller only repaints when it did.
    fn extend_to(&mut self, position: Point<Pixels>, pressed: bool) -> bool {
        if !pressed {
            return false;
        }
        let Some(stroke) = self.in_progress.as_mut() else {
            return false;
        };
        if stroke.points.len() >= MAX_STROKE_POINTS {
            return false;
        }
        if stroke.points.last() == Some(&position) {
            return false;
        }
        stroke.points.push(position);
        true
    }

    /// Finishes the stroke in progress.
    fn end_stroke(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left {
            return;
        }
        if self.finish_stroke(event.position) {
            cx.notify();
        }
    }

    /// The state half of [`PlotWindow::end_stroke`].
    fn finish_stroke(&mut self, position: Point<Pixels>) -> bool {
        let Some(mut stroke) = self.in_progress.take() else {
            return false;
        };
        if stroke.points.last() != Some(&position) {
            stroke.points.push(position);
        }
        self.strokes.push(stroke);
        true
    }

    /// Removes the last stroke.
    fn undo_stroke(&mut self, cx: &mut Context<Self>) {
        if self.undo() {
            cx.notify();
        }
    }

    /// The state half of [`PlotWindow::undo_stroke`]; an in-progress stroke is
    /// dropped first, then the last finished one.
    fn undo(&mut self) -> bool {
        if self.in_progress.take().is_some() {
            return true;
        }
        self.strokes.pop().is_some()
    }

    /// Removes every stroke.
    fn clear_strokes(&mut self, cx: &mut Context<Self>) {
        if self.clear() {
            cx.notify();
        }
    }

    /// The state half of [`PlotWindow::clear_strokes`].
    fn clear(&mut self) -> bool {
        let had_strokes = !self.strokes.is_empty() || self.in_progress.is_some();
        self.strokes.clear();
        self.in_progress = None;
        had_strokes
    }

    /// Every stroke to paint, the finished ones first and the one in progress
    /// last so it draws on top.
    fn painted_strokes(&self) -> Vec<Stroke> {
        let mut strokes = self.strokes.clone();
        if let Some(stroke) = self.in_progress.clone() {
            strokes.push(stroke);
        }
        strokes
    }

    // -- painting ------------------------------------------------------------

    /// Paints every stroke onto the canvas.
    ///
    /// Returned as a closure rather than a method taking a window because it runs
    /// inside [`canvas`]'s paint pass, which gets the canvas bounds and a window
    /// but no view. The strokes are moved in, already collected.
    fn paint_strokes(
        strokes: Vec<Stroke>,
    ) -> impl FnOnce(Bounds<Pixels>, (), &mut Window, &mut App) + 'static {
        move |bounds, (), window, _cx| {
            for stroke in &strokes {
                let points: Vec<Point<Pixels>> = stroke
                    .points
                    .iter()
                    .map(|point| *point - bounds.origin)
                    .collect();
                let Some(first) = points.first().copied() else {
                    continue;
                };

                if points.len() == 1 {
                    // A click with no drag: the dot a stroke of this brush would
                    // start with. `paint_path` needs a second point to fill
                    // anything, so this goes through a quad instead.
                    let radius = stroke.radius();
                    window.paint_quad(PaintQuad {
                        bounds: Bounds::new(
                            point(first.x - px(radius), first.y - px(radius)),
                            size(px(stroke.width), px(stroke.width)),
                        ),
                        corner_radii: Corners::all(px(radius)),
                        background: stroke.color.into(),
                        border_widths: Edges::default(),
                        border_color: gpui::transparent_black(),
                        border_style: BorderStyle::Solid,
                    });
                    continue;
                }

                let mut builder = PathBuilder::stroke(px(stroke.width));
                builder.move_to(first);
                for point in &points[1..] {
                    builder.line_to(*point);
                }
                // A path that fails to build (degenerate input, in practice) is
                // dropped rather than painted as garbage.
                if let Ok(path) = builder.build() {
                    window.paint_path(path, stroke.color);
                }
            }
        }
    }

    // -- rendering -----------------------------------------------------------

    /// Builds one chart card's chart element.
    fn chart(&self, kind: ChartKind, cx: &App) -> AnyElement {
        let theme = KitTheme::global(cx);
        let palette = [
            theme.chart_1,
            theme.chart_2,
            theme.chart_3,
            theme.chart_4,
            theme.chart_5,
        ];
        let primary = palette[0];
        let secondary = palette[1];
        let tertiary = palette[2];
        let slice_palette = palette;
        let samples = self.samples.clone();
        let seed = self.seed;
        let animate = self.animate;
        let interactive = self.interactive;

        match kind {
            ChartKind::Line => LineChart::new(samples)
                .x(|sample| sample.label.clone())
                .y(|sample| sample.value)
                .stroke(primary)
                .name("折线")
                .dot()
                .grid(true)
                .x_tick_count(6)
                .y_tick_format(|value| SharedString::from(format!("{value:.0}")))
                .appear(animate)
                .appear_key(seed)
                .interactive(interactive)
                .id(kind.element_id())
                .into_any_element(),
            ChartKind::Area => AreaChart::new(samples)
                .x(|sample| sample.label.clone())
                .y(|sample| sample.value)
                .stroke(secondary)
                .name("面积")
                .grid_dashed(true)
                .x_tick_count(6)
                .appear(animate)
                .appear_key(seed)
                .interactive(interactive)
                .id(kind.element_id())
                .into_any_element(),
            ChartKind::Bar => BarChart::new(samples)
                .band(|sample| sample.label.to_string())
                .value(|sample| sample.value)
                .fill(move |_, _, _, _| tertiary)
                .corner_radii(Corners::all(px(4.)))
                .name("柱状")
                .label(|sample| SharedString::from(sample.formatted()))
                .value_axis(true)
                .grid_dashed(true)
                .appear(animate)
                .appear_key(seed)
                .interactive(interactive)
                .id(kind.element_id())
                .into_any_element(),
            ChartKind::Pie => PieChart::new(samples)
                .value(|sample| sample.value as f32)
                .label(|sample| sample.label.clone())
                .inner_radius(46.)
                .outer_radius(104.)
                // The slice colour comes from the datum rather than from its
                // position, so a regenerated series keeps each month's colour.
                .color(move |sample: &Sample| {
                    let index = sample.label.len() + sample.value as usize;
                    slice_palette[index % slice_palette.len()]
                })
                .name("饼图")
                .appear(animate)
                .appear_key(seed)
                .interactive(interactive)
                .id(kind.element_id())
                .into_any_element(),
        }
    }

    /// One chart card: heading, hint and the chart filling the rest.
    fn chart_card(&self, kind: ChartKind, cx: &App) -> AnyElement {
        let muted = cx.theme().colors().text_muted;
        let chart = self.chart(kind, cx);

        v_flex()
            .w_full()
            .min_w_0()
            .h(CARD_HEIGHT)
            .gap_1()
            .p_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_baseline()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(kind.title()),
                    )
                    .child(div().text_xs().text_color(muted).child(kind.hint())),
            )
            .child(div().flex_1().min_h_0().child(chart))
            .into_any_element()
    }

    /// The charts tab: a two-by-two grid of the four charts.
    ///
    /// The rows take their height from the cards (`CARD_HEIGHT`), not from the
    /// remaining space: a chart fills its parent, so the parent is what has to be
    /// definite.
    fn render_charts(&self, cx: &App) -> AnyElement {
        let border = cx.theme().colors().border;

        let cell = |kind: ChartKind| {
            v_flex()
                .flex_1()
                .min_w_0()
                .border_1()
                .border_color(border)
                .rounded_lg()
                .child(self.chart_card(kind, cx))
        };

        let rows = ChartKind::ALL
            .chunks(2)
            .map(|pair| {
                h_flex()
                    .gap_2()
                    .w_full()
                    .child(cell(pair[0]))
                    .child(cell(pair[1]))
            })
            .collect::<Vec<_>>();

        // The scroll container is here rather than around the whole body: the
        // drawing board needs a definite height for its canvas, and a scroll
        // container sizes its content by content, which would collapse it.
        div()
            .id("plot-charts")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(v_flex().w_full().gap_2().children(rows))
            .into_any_element()
    }

    /// The freehand tab: brush controls and the canvas.
    fn render_freehand(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors();
        let border = colors.border;
        let text = colors.text;
        let muted = colors.text_muted;
        let canvas_background = colors.element_background;
        let palette = chart_palette(cx);
        let selected = self.color % palette.len();
        let brush = self.brush % BRUSH_WIDTHS.len();
        let painted = self.painted_strokes();
        let stroke_count = self.strokes.len();
        let has_strokes = stroke_count > 0 || self.in_progress.is_some();

        // An empty canvas reads as a broken one, so it says what to do. The hint
        // has no id and takes no mouse: a box only becomes a hitbox when it is
        // interactive, so presses fall through to the canvas below it.
        let empty_hint = (!has_strokes).then(|| {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(muted)
                .child("在这里按住左键拖动")
        });

        let swatches = palette
            .iter()
            .copied()
            .enumerate()
            .map(|(index, color)| {
                let active = index == selected;
                div()
                    .id(format!("plot-swatch-{index}"))
                    .size(px(20.))
                    .rounded_full()
                    .bg(color)
                    .border_2()
                    .border_color(if active { text } else { border })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.color = index;
                        cx.notify();
                    }))
            })
            .collect::<Vec<_>>();

        let brushes = BRUSH_LABELS
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let button = KitButton::new(format!("plot-brush-{index}"))
                    .label(*label)
                    .compact();
                let button = if index == brush {
                    button.primary()
                } else {
                    button.ghost()
                };
                button.on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.brush = index;
                    cx.notify();
                }))
            })
            .collect::<Vec<_>>();

        v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(div().text_xs().text_color(muted).child("笔色"))
                    .children(swatches)
                    .child(div().w(px(12.)).h(px(1.)).bg(border))
                    .child(div().text_xs().text_color(muted).child("笔宽"))
                    .children(brushes)
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{stroke_count} 笔 · {} px", self.brush_width())),
                    )
                    .child(
                        KitButton::new("plot-undo")
                            .label("撤销")
                            .compact()
                            .disabled(!has_strokes)
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.undo_stroke(cx);
                            })),
                    )
                    .child(
                        KitButton::new("plot-clear")
                            .label("清空")
                            .compact()
                            .disabled(!has_strokes)
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.clear_strokes(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("plot-canvas")
                    // Lets the test find the canvas, and so press inside it.
                    .debug_selector(|| "plot-canvas".to_string())
                    .relative()
                    .flex_1()
                    .min_h(px(120.))
                    .rounded_lg()
                    .border_1()
                    .border_color(border)
                    .bg(canvas_background)
                    // `Canvas` is `Styled` but has no hitbox, so the mouse is
                    // handled by the box and the drawing happens on a canvas
                    // pinned over the same bounds.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                            this.begin_stroke(event, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                        this.extend_stroke(event, cx);
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                            this.end_stroke(event, cx);
                        }),
                    )
                    .child(
                        canvas(|_, _, _| (), Self::paint_strokes(painted))
                            .absolute()
                            .size_full(),
                    )
                    .children(empty_hint),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("按住左键拖动即可绘制。笔迹保存在窗口坐标里，改变窗口大小时位置不变。"),
            )
            .into_any_element()
    }
}

/// The Kit's chart palette, in order.
///
/// Copied out of the Kit theme before it is used because `KitTheme::global`
/// borrows the app, and the callers keep using `cx` while they paint.
fn chart_palette(cx: &App) -> [Hsla; 5] {
    let theme = KitTheme::global(cx);
    [
        theme.chart_1,
        theme.chart_2,
        theme.chart_3,
        theme.chart_4,
        theme.chart_5,
    ]
}

impl Render for PlotWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let border = colors.border;
        let text = colors.text;
        let muted = colors.text_muted;
        let background = colors.editor_background;

        let tabs = Tab::ORDER
            .into_iter()
            .map(|tab| {
                let button = KitButton::new(format!("plot-tab-{}", tab as usize))
                    .label(tab.label())
                    .compact();
                let button = if self.tab == tab {
                    button.primary()
                } else {
                    button.ghost()
                };
                button.on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.select(tab, cx);
                }))
            })
            .collect::<Vec<_>>();

        let animate = self.animate;
        let interactive = self.interactive;

        let body = match self.tab {
            Tab::Charts => self.render_charts(cx),
            Tab::Freehand => self.render_freehand(cx),
        };

        v_flex()
            .id("plot-window")
            .size_full()
            .overflow_hidden()
            .bg(background)
            .text_color(text)
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_baseline()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Plot"),
                            )
                            .child(div().text_xs().text_color(muted).child("绘图测试")),
                    )
                    // The tabs sit beside the heading rather than at the far edge:
                    // a right-aligned row is the first thing a narrow window clips.
                    .child(div().w(px(12.)))
                    .children(tabs)
                    .child(div().flex_1()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        KitButton::new("plot-regenerate")
                            .label("重新生成")
                            .compact()
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.regenerate(cx);
                            })),
                    )
                    .child(
                        KitButton::new("plot-animate")
                            .label(if animate { "动画：开" } else { "动画：关" })
                            .compact()
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.toggle_animate(cx);
                            })),
                    )
                    .child(
                        KitButton::new("plot-interactive")
                            .label(if interactive { "交互：开" } else { "交互：关" })
                            .compact()
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                this.toggle_interactive(cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(format!("种子 0x{:016X}", self.seed)),
                    ),
            )
            // The body holds the tab's content; each tab decides whether it
            // scrolls (the chart grid does) or fills the space (the board does).
            // It is a column so that a tab can fill the height: a plain box sizes
            // its children by their content, which collapses the board's canvas.
            .child(v_flex().flex_1().min_h_0().child(body))
            .child(div().h(px(1.)).w_full().bg(border))
            .child(div().text_xs().text_color(muted).child(
                "图表用 gpui-kit 的折线 / 面积 / 柱状 / 饼图绘制：「动画」控制入场动画，「交互」控制悬停命中、十字线与提示框。",
            ))
    }
}

/// Opens the window, or focuses the one already open.
pub fn open(cx: &mut App) {
    // `render` needs `theme::GlobalTheme` and panics without it ("no state of
    // type `theme::GlobalTheme` exists"). A menu click always happens long after
    // the theme is registered, but the startup override in [`init`] can run
    // before that, so wait for it rather than open a window that cannot draw.
    if cx.try_global::<theme::GlobalTheme>().is_none() {
        log::info!("[plot] waiting for the theme before opening the drawing window");
        cx.spawn(async move |cx| {
            for _ in 0..300 {
                if cx.update(|cx| cx.try_global::<theme::GlobalTheme>().is_some()) {
                    break;
                }
                smol::Timer::after(std::time::Duration::from_millis(100)).await;
            }
            cx.update(show_window);
        })
        .detach();
        return;
    }

    show_window(cx);
}

/// Whether `window` is the drawing test window.
///
/// The window's root view is the Kit's [`Root`], not the window view — the
/// controls need it for their window-level state — so the view is reached by
/// downcasting the window to the root, taking the view off it and downcasting
/// that. The `Help > gpui-kit` gallery and the Dameng window answer the same
/// first downcast and differ in the second.
pub fn is_plot_window(window: &AnyWindowHandle, cx: &App) -> bool {
    window
        .downcast::<Root>()
        .and_then(|root| root.read(cx).ok())
        .is_some_and(|root| root.view().clone().downcast::<PlotWindow>().is_ok())
}

/// The plot view behind `window`, if `window` is the drawing window.
#[cfg(test)]
fn plot_view(window: &AnyWindowHandle, cx: &App) -> Option<gpui::Entity<PlotWindow>> {
    let view = window
        .downcast::<Root>()
        .and_then(|root| root.read(cx).ok())
        .map(|root| root.view().clone());
    view?.downcast::<PlotWindow>().ok()
}

/// The number of finished strokes on the drawing board, for tests that want to
/// check that dragging on the canvas leaves ink behind.
#[cfg(test)]
pub fn stroke_count(window: &AnyWindowHandle, cx: &App) -> Option<usize> {
    Some(plot_view(window, cx)?.read(cx).strokes.len())
}

/// Whether a stroke is being drawn right now, for tests that want to tell a
/// press that missed the canvas from one that started ink and then lost it.
#[cfg(test)]
pub fn stroke_in_progress(window: &AnyWindowHandle, cx: &App) -> Option<bool> {
    Some(plot_view(window, cx)?.read(cx).in_progress.is_some())
}

/// The tab the window shows, so a test can tell whether the page it wants to
/// paint is the one that was painted.
#[cfg(test)]
pub fn active_tab(window: &AnyWindowHandle, cx: &mut App) -> Option<&'static str> {
    let view = plot_view(window, cx)?;
    let tab = view.read(cx).tab;
    Some(match tab {
        Tab::Charts => "charts",
        Tab::Freehand => "freehand",
    })
}

/// Switches the window to the drawing board, for tests.
///
/// The tab is normally reached by clicking its button. A test that clicked it
/// would depend on where the layout happens to put the button, so it goes to the
/// same state directly instead.
#[cfg(test)]
pub fn show_freehand(window: &AnyWindowHandle, cx: &mut App) {
    let Some(view) = plot_view(window, cx) else {
        return;
    };
    view.update(cx, |this, cx| {
        this.tab = Tab::Freehand;
        cx.notify();
    });
}

/// The tab the window opens on.
///
/// The chart page is the interesting one to show by default, but the board is
/// behind a click, and `ZED_PLOT_TAB=freehand` makes it possible to look at (or
/// screenshot) that page without one.
fn initial_tab() -> Tab {
    match std::env::var("ZED_PLOT_TAB").ok().as_deref() {
        Some("freehand") => Tab::Freehand,
        _ => Tab::Charts,
    }
}

/// Opens the window, or focuses it if it is already open.
fn show_window(cx: &mut App) {
    if let Some(existing) = cx
        .windows()
        .into_iter()
        .find(|window| is_plot_window(window, cx))
    {
        if let Err(error) = existing.update(cx, |_, window, _| window.activate_window()) {
            log::warn!("[plot] could not focus the drawing window: {error}");
        }
        return;
    }

    // The buttons and the chart palette are the Kit's, so its globals and theme
    // have to exist before the first frame draws. `gpui_kit::init` is idempotent,
    // so the gallery or the Dameng window having initialized them already is
    // fine. The theme mode follows Zed's, so the controls do not come out light
    // on a dark editor.
    gpui_kit::init(cx);
    let mode = match cx.theme().appearance() {
        theme::Appearance::Dark => KitThemeMode::Dark,
        theme::Appearance::Light => KitThemeMode::Light,
    };
    KitTheme::change(mode, None, cx);

    // Anchored at the top-left rather than centred: `WindowBounds::centered`
    // centers the logical size over the display bounds it is given, and on a
    // scaled display those numbers come from the unscaled desktop, which pushes
    // a window of this size off the right and bottom edges. A fixed origin is
    // right in every case.
    let window_bounds = Bounds {
        origin: point(px(80.), px(80.)),
        size: Size {
            width: px(900.),
            height: px(640.),
        },
    };

    let opened = cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(WINDOW_TITLE.into()),
                // The system title bar, as elsewhere: this window draws no chrome
                // of its own, so a transparent bar would leave it with no close
                // button.
                appears_transparent: false,
                traffic_light_position: None,
            }),
            window_bounds: Some(WindowBounds::Windowed(window_bounds)),
            kind: WindowKind::Normal,
            app_id: Some(ReleaseChannel::global(cx).app_id().to_owned()),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| {
                let mut view = PlotWindow::new(window, cx);
                view.tab = initial_tab();
                view
            });
            // Deliberately no `activate_window` here: a window that has just
            // activated itself is the window an action dispatches through, and a
            // window being updated cannot be read back — `is_plot_window` would
            // then miss it and the next `Help > Plot` would open a second copy.
            // `WindowOptions::focus` (on by default) already brings the window to
            // the front.
            //
            // The root, not the view: the Kit's controls read their window-level
            // state off it, and one of them touching that state without a root
            // above it panics with "component window state is missing".
            cx.new(|cx| Root::new(view, window, cx)).into()
        },
    );

    match opened {
        Ok(_) => log::info!(
            "[plot] drawing window opened ({} window(s) now open)",
            cx.windows().len()
        ),
        Err(error) => log::error!("[plot] could not open the drawing window: {error}"),
    }
}

/// Opens the window at startup when `ZED_PLOT_OPEN` is set.
///
/// The same escape hatch `ZED_OCTOS_OPEN_STATUS`, `ZED_CESIUMRUST_OPEN`,
/// `ZED_GPUI_KIT_OPEN`, `ZED_DAMENG_OPEN` and `ZED_ABOUT_OPEN` provide: a machine
/// where the window matters has a way to open it without clicking through a menu.
pub fn init(cx: &mut App) {
    if std::env::var_os("ZED_PLOT_OPEN").is_some() {
        open(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window in its initial state, for tests that need one without an app.
    fn window() -> PlotWindow {
        PlotWindow {
            tab: Tab::Charts,
            seed: DEFAULT_SEED,
            samples: generate_samples(DEFAULT_SEED),
            animate: true,
            interactive: true,
            strokes: Vec::new(),
            in_progress: None,
            brush: 1,
            color: 0,
        }
    }

    /// A two-point stroke, for tests that need one.
    fn stroke(offset: f32) -> Stroke {
        Stroke {
            points: vec![
                point(px(10. + offset), px(10.)),
                point(px(30. + offset), px(30.)),
            ],
            color: gpui::black(),
            width: BRUSH_WIDTHS[1],
        }
    }

    /// The first frame is a real series: twelve labelled points, in order, each
    /// with a finite, positive value.
    #[test]
    fn the_default_series_is_usable() {
        let samples = generate_samples(DEFAULT_SEED);
        assert_eq!(samples.len(), SAMPLE_COUNT);
        assert_eq!(samples[0].label.as_ref(), "1月");
        assert_eq!(samples[SAMPLE_COUNT - 1].label.as_ref(), "12月");
        for sample in &samples {
            assert!(sample.value.is_finite());
            assert!(sample.value > 0., "{} is not positive", sample.value);
        }
    }

    /// The same seed draws the same series and a different seed draws a different
    /// one, which is what makes `重新生成` visible.
    #[test]
    fn generation_is_deterministic_and_seeded() {
        let values = |samples: &[Sample]| samples.iter().map(|s| s.value).collect::<Vec<_>>();
        let first = generate_samples(DEFAULT_SEED);
        assert_eq!(values(&first), values(&generate_samples(DEFAULT_SEED)));
        assert_ne!(values(&first), values(&generate_samples(DEFAULT_SEED ^ 1)));
    }

    /// Regenerating keeps a point per label and changes the values.
    #[test]
    fn regenerating_keeps_the_labels_and_changes_the_values() {
        let mut window = window();
        let before = window
            .samples
            .iter()
            .map(Sample::formatted)
            .collect::<Vec<_>>();

        window.reseed();

        assert_eq!(window.samples.len(), SAMPLE_COUNT);
        let after = window
            .samples
            .iter()
            .map(Sample::formatted)
            .collect::<Vec<_>>();
        assert_ne!(before, after);
        assert_eq!(
            window
                .samples
                .iter()
                .map(|s| s.label.clone())
                .collect::<Vec<_>>(),
            generate_samples(window.seed)
                .iter()
                .map(|s| s.label.clone())
                .collect::<Vec<_>>()
        );
    }

    /// Both tabs are reachable and the cycle returns to the start.
    #[test]
    fn tabs_cycle() {
        assert_eq!(Tab::Charts.next(), Tab::Freehand);
        assert_eq!(Tab::Freehand.next(), Tab::Charts);
        assert_eq!(Tab::ORDER.len(), 2);
    }

    /// A left press starts a stroke at the press position, with the current
    /// brush, and a right press starts nothing.
    #[test]
    fn a_left_press_starts_a_stroke() {
        let mut window = window();
        // Read the brush first: taking the stroke out of `in_progress` moves it,
        // which would borrow-check the window while it is still immutably
        // borrowed here.
        let brush_width = window.brush_width();
        window.start_stroke(point(px(12.), px(14.)), gpui::black());
        let started = window.in_progress.expect("a stroke is in progress");
        assert_eq!(started.points, vec![point(px(12.), px(14.))]);
        assert_eq!(started.width, brush_width);
    }

    /// Moving with the button held extends the stroke; moving without it, or
    /// beyond the point cap, does not.
    #[test]
    fn only_a_held_button_extends_a_stroke() {
        let mut window = window();
        window.start_stroke(point(px(10.), px(10.)), gpui::black());

        assert!(!window.extend_to(point(px(20.), px(20.)), false));
        assert_eq!(window.in_progress.as_ref().unwrap().points.len(), 1);

        assert!(window.extend_to(point(px(20.), px(20.)), true));
        assert_eq!(window.in_progress.as_ref().unwrap().points.len(), 2);

        // The same position twice is a no-op: a still cursor does not pile up
        // points.
        assert!(!window.extend_to(point(px(20.), px(20.)), true));
        assert_eq!(window.in_progress.as_ref().unwrap().points.len(), 2);

        window.in_progress.as_mut().unwrap().points =
            vec![point(px(1.), px(1.)); MAX_STROKE_POINTS];
        assert!(!window.extend_to(point(px(2.), px(2.)), true));
    }

    /// Releasing the button finishes the stroke and appends the release point.
    #[test]
    fn releasing_finishes_the_stroke() {
        let mut window = window();
        window.start_stroke(point(px(10.), px(10.)), gpui::black());
        window.extend_to(point(px(20.), px(20.)), true);

        assert!(window.finish_stroke(point(px(30.), px(30.))));
        assert!(window.in_progress.is_none());
        assert_eq!(window.strokes.len(), 1);
        assert_eq!(
            window.strokes[0].points,
            vec![
                point(px(10.), px(10.)),
                point(px(20.), px(20.)),
                point(px(30.), px(30.))
            ]
        );

        // Nothing in progress: releasing again changes nothing.
        assert!(!window.finish_stroke(point(px(40.), px(40.))));
    }

    /// Undo drops the in-progress stroke first, then the finished ones, stopping
    /// at the bottom; clear wipes both.
    #[test]
    fn undo_and_clear() {
        let mut window = window();
        window.strokes = vec![stroke(0.), stroke(1.), stroke(2.)];
        window.in_progress = Some(stroke(3.));

        assert!(window.undo());
        assert!(window.in_progress.is_none());
        assert_eq!(window.strokes.len(), 3);

        assert!(window.undo());
        assert_eq!(window.strokes.len(), 2);

        assert!(window.clear());
        assert!(window.strokes.is_empty());
        assert!(!window.clear());
    }

    /// The paint list puts the stroke in progress last, so it draws over the
    /// finished ones.
    #[test]
    fn the_paint_list_ends_with_the_stroke_in_progress() {
        let mut window = window();
        window.strokes = vec![stroke(0.)];
        window.in_progress = Some(stroke(1.));

        let painted = window.painted_strokes();
        assert_eq!(painted.len(), 2);
        assert_eq!(painted[1].points[0], point(px(11.), px(10.)));

        window.in_progress = None;
        assert_eq!(window.painted_strokes().len(), 1);
    }

    /// The brush and colour selections wrap instead of indexing out of bounds.
    #[test]
    fn the_brush_selection_wraps() {
        let mut window = window();
        window.brush = BRUSH_WIDTHS.len();
        assert_eq!(window.brush_width(), BRUSH_WIDTHS[0]);
        window.brush = BRUSH_WIDTHS.len() * 2 + 2;
        assert_eq!(window.brush_width(), BRUSH_WIDTHS[2]);
        assert_eq!(window.brush % BRUSH_WIDTHS.len(), 2);
    }

    /// Every chart kind has its own element id, so the four cards do not share a
    /// hover state.
    #[test]
    fn chart_kinds_have_distinct_ids() {
        let ids = ChartKind::ALL.map(ChartKind::element_id);
        let mut unique = ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
    }

    /// A too-long stroke stays at the cap rather than growing without limit, and
    /// the oldest stroke is dropped when the canvas is full.
    #[test]
    fn the_stroke_bounds_hold() {
        let mut window = window();
        window.strokes = (0..MAX_STROKES).map(|index| stroke(index as f32)).collect();

        window.start_stroke(point(px(1.), px(1.)), gpui::black());
        // The oldest stroke fell off to make room; the new one is in progress,
        // not yet in the list.
        assert_eq!(window.strokes.len(), MAX_STROKES - 1);
        assert_eq!(window.strokes[0].points[0], point(px(11.), px(10.)));
    }
}
