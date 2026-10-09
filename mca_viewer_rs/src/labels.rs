use parley::Layout;
use vello::kurbo;
#[derive(Default)]
pub struct LabelCtx {
    font_ctx: parley::FontContext,
    layout_ctx: parley::LayoutContext<()>,
}

impl LabelCtx {
    // NOTE: currently have scale hardcoded to 1., ui scaling is done with the affine transform
    // need to integrate display scale too
    pub fn build_label(&mut self, text: String, font_size: f32) -> Label {
        // pretty sure quantization is not beneficial and possibly slightly harmful, given that i
        // render through vello and an affine transform and whatever positions the layout actually
        // produces will not be rendered to a pixel-grid straight away
        //
        // text already looks good now, but maybe it could be optimized by not applying the
        // scale component of the affine transform to the produced glyphs, instead doing the layout at
        // the correct scale (would require recomputing layouts whenever the scale changes), WITH
        // quantization, then apply only the integral part of the affines translation (in screen
        // space) to the position to preserve the quantization?
        // also look into DrawGlyphs::hint?
        let mut builder =
            self.layout_ctx
                .ranged_builder(&mut self.font_ctx, text.as_str(), 1., false);
        builder.push_default(parley::StyleProperty::FontSize(font_size));

        // setting line height to 1. to get the reported width/height be a tighter bounding box
        // around the text, should tweak if i ever use labels for anything with more than one line
        builder.push_default(parley::StyleProperty::LineHeight(
            parley::LineHeight::FontSizeRelative(1.),
        ));
        let mut layout: Layout<()> = builder.build(text.as_str());
        layout.break_all_lines(None);
        Label { layout, text }
    }
}

pub struct Label {
    layout: parley::Layout<()>,
    // keeping original string with the label so they can be cached and invalidated if a points name
    // changes
    text: String,
}

impl Label {
    pub fn width(&self) -> f64 {
        f64::from(self.layout.width())
    }

    pub fn height(&self) -> f64 {
        f64::from(self.layout.height())
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn render_text(
        &self,
        scene: &mut vello::Scene,
        pos: impl Into<kurbo::Point>,
        color: vello::peniko::color::AlphaColor<vello::peniko::color::Srgb>,
    ) {
        let pos = pos.into();
        for line in self.layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(g) = item {
                    scene
                        .draw_glyphs(g.run().font())
                        .font_size(g.run().font_size())
                        .normalized_coords(g.run().normalized_coords())
                        .brush(color)
                        .transform(kurbo::Affine::translate((pos.x, pos.y)))
                        .draw(
                            vello::peniko::Fill::NonZero,
                            g.positioned_glyphs().map(|g| vello::Glyph {
                                id: g.id,
                                x: g.x,
                                y: g.y,
                            }),
                        );
                }
            }
        }
    }
}
