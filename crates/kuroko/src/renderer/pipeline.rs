use crate::core::{ColorPrimaries, TransferFunction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRange {
    Unspecified,
    Limited,
    Full,
}

impl Default for ColorRange {
    fn default() -> Self {
        Self::Unspecified
    }
}

impl ColorRange {
    pub fn resolve(self, fallback: Self) -> Self {
        match self {
            Self::Unspecified => fallback,
            _ => self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixCoefficients {
    Unspecified,
    Identity,
    Bt601,
    Bt709,
    Bt2020NonConstantLuminance,
}

impl Default for MatrixCoefficients {
    fn default() -> Self {
        Self::Unspecified
    }
}

impl MatrixCoefficients {
    pub fn resolve(self, primaries: ColorPrimaries) -> Self {
        if self != Self::Unspecified {
            return self;
        }
        match primaries {
            ColorPrimaries::Bt2020 => Self::Bt2020NonConstantLuminance,
            ColorPrimaries::Bt709 | ColorPrimaries::DisplayP3 => Self::Bt709,
            ColorPrimaries::Unknown => Self::Bt709,
        }
    }

    pub fn luma_coefficients(self, primaries: ColorPrimaries) -> LumaCoefficients {
        match self.resolve(primaries) {
            Self::Bt601 => LumaCoefficients::new(0.2990, 0.5870, 0.1140),
            Self::Bt2020NonConstantLuminance => LumaCoefficients::new(0.2627, 0.6780, 0.0593),
            Self::Identity | Self::Bt709 | Self::Unspecified => {
                LumaCoefficients::new(0.2126, 0.7152, 0.0722)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LumaCoefficients {
    pub kr: f32,
    pub kg: f32,
    pub kb: f32,
}

impl LumaCoefficients {
    pub const fn new(kr: f32, kg: f32, kb: f32) -> Self {
        Self { kr, kg, kb }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneMapOperator {
    Clip,
    Reinhard,
    Mobius,
}

impl Default for ToneMapOperator {
    fn default() -> Self {
        Self::Mobius
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalerKernel {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos3,
}

impl Default for ScalerKernel {
    fn default() -> Self {
        Self::Bilinear
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceColorState {
    pub primaries: ColorPrimaries,
    pub transfer: TransferFunction,
    pub matrix: MatrixCoefficients,
    pub range: ColorRange,
    pub nominal_peak_nits: f32,
    pub reference_white_nits: f32,
}

impl SourceColorState {
    pub fn new(primaries: ColorPrimaries, transfer: TransferFunction) -> Self {
        Self {
            primaries,
            transfer,
            matrix: MatrixCoefficients::default(),
            range: ColorRange::default(),
            nominal_peak_nits: nominal_peak_for_transfer(transfer),
            reference_white_nits: 203.0,
        }
    }

    pub fn range(mut self, range: ColorRange) -> Self {
        self.range = range;
        self
    }

    pub fn matrix(mut self, matrix: MatrixCoefficients) -> Self {
        self.matrix = matrix;
        self
    }

    pub fn nominal_peak_nits(mut self, peak: f32) -> Self {
        self.nominal_peak_nits = peak.max(1.0);
        self
    }

    pub fn reference_white_nits(mut self, white: f32) -> Self {
        self.reference_white_nits = white.max(1.0);
        self
    }

    pub fn is_hdr(&self) -> bool {
        matches!(self.transfer, TransferFunction::Pq | TransferFunction::Hlg)
            || self.nominal_peak_nits > self.reference_white_nits * 1.5
    }
}

impl Default for SourceColorState {
    fn default() -> Self {
        Self::new(ColorPrimaries::Unknown, TransferFunction::Unknown)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetColorState {
    pub primaries: ColorPrimaries,
    pub transfer: TransferFunction,
    pub peak_nits: f32,
    pub reference_white_nits: f32,
    pub edr_headroom: f32,
}

impl TargetColorState {
    pub fn sdr(primaries: ColorPrimaries) -> Self {
        Self {
            primaries,
            transfer: TransferFunction::Srgb,
            peak_nits: 100.0,
            reference_white_nits: 100.0,
            edr_headroom: 1.0,
        }
    }

    pub fn apple_edr(primaries: ColorPrimaries, headroom: f32) -> Self {
        let headroom = headroom.max(1.0);
        Self {
            primaries,
            transfer: TransferFunction::Srgb,
            peak_nits: 203.0 * headroom,
            reference_white_nits: 203.0,
            edr_headroom: headroom,
        }
    }
}

impl Default for TargetColorState {
    fn default() -> Self {
        Self::sdr(ColorPrimaries::Bt709)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneMapConfig {
    pub operator: ToneMapOperator,
    pub knee_start: f32,
    pub desaturate: f32,
}

impl Default for ToneMapConfig {
    fn default() -> Self {
        Self {
            operator: ToneMapOperator::Mobius,
            knee_start: 0.75,
            desaturate: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScalerConfig {
    pub kernel: ScalerKernel,
    pub radius: f32,
}

impl Default for ScalerConfig {
    fn default() -> Self {
        Self {
            kernel: ScalerKernel::Bilinear,
            radius: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderPassKind {
    ImportFrame,
    PlaneSampling,
    ChromaReconstruction,
    TransferDecode,
    GamutMap,
    ToneMap,
    Scale,
    OverlayComposite,
    Dither,
    OutputTransform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPass {
    pub kind: RenderPassKind,
    pub label: &'static str,
}

impl RenderPass {
    pub const fn new(kind: RenderPassKind, label: &'static str) -> Self {
        Self { kind, label }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderGraph {
    passes: Vec<RenderPass>,
}

impl RenderGraph {
    pub fn new() -> Self {
        Self { passes: Vec::new() }
    }

    pub fn push(&mut self, pass: RenderPass) {
        self.passes.push(pass);
    }

    pub fn passes(&self) -> &[RenderPass] {
        &self.passes
    }

    pub fn contains(&self, kind: RenderPassKind) -> bool {
        self.passes.iter().any(|pass| pass.kind == kind)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoRenderPipeline {
    pub source: SourceColorState,
    pub target: TargetColorState,
    pub tone_map: ToneMapConfig,
    pub scaler: ScalerConfig,
    pub graph: RenderGraph,
}

impl VideoRenderPipeline {
    pub fn new(source: SourceColorState, target: TargetColorState) -> Self {
        let tone_map = ToneMapConfig::default();
        let scaler = ScalerConfig::default();
        let graph = build_graph(source, target, scaler);
        Self {
            source,
            target,
            tone_map,
            scaler,
            graph,
        }
    }

    pub fn sdr_default() -> Self {
        Self::new(SourceColorState::default(), TargetColorState::default())
    }

    pub fn requires_tone_mapping(&self) -> bool {
        requires_tone_mapping(self.source, self.target)
    }

    pub fn luma_coefficients(&self) -> LumaCoefficients {
        self.source.matrix.luma_coefficients(self.source.primaries)
    }
}

impl Default for VideoRenderPipeline {
    fn default() -> Self {
        Self::sdr_default()
    }
}

fn build_graph(
    source: SourceColorState,
    target: TargetColorState,
    scaler: ScalerConfig,
) -> RenderGraph {
    let mut graph = RenderGraph::new();
    graph.push(RenderPass::new(RenderPassKind::ImportFrame, "import frame"));
    graph.push(RenderPass::new(
        RenderPassKind::PlaneSampling,
        "sample YCbCr planes",
    ));
    graph.push(RenderPass::new(
        RenderPassKind::ChromaReconstruction,
        "reconstruct chroma",
    ));
    graph.push(RenderPass::new(
        RenderPassKind::TransferDecode,
        "decode transfer function",
    ));
    if source.primaries != target.primaries && source.primaries != ColorPrimaries::Unknown {
        graph.push(RenderPass::new(RenderPassKind::GamutMap, "map gamut"));
    }
    if requires_tone_mapping(source, target) {
        graph.push(RenderPass::new(RenderPassKind::ToneMap, "tone map"));
    }
    if scaler.kernel != ScalerKernel::Nearest {
        graph.push(RenderPass::new(RenderPassKind::Scale, "scale"));
    }
    graph.push(RenderPass::new(
        RenderPassKind::OverlayComposite,
        "composite overlays",
    ));
    graph.push(RenderPass::new(RenderPassKind::Dither, "dither"));
    graph.push(RenderPass::new(
        RenderPassKind::OutputTransform,
        "output transform",
    ));
    graph
}

fn requires_tone_mapping(source: SourceColorState, target: TargetColorState) -> bool {
    if !source.is_hdr() {
        return false;
    }
    source.nominal_peak_nits > target.peak_nits * 1.05
}

fn nominal_peak_for_transfer(transfer: TransferFunction) -> f32 {
    match transfer {
        TransferFunction::Pq => 1000.0,
        TransferFunction::Hlg => 1000.0,
        TransferFunction::Srgb | TransferFunction::Bt1886 => 100.0,
        TransferFunction::Unknown => 100.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdr_pq_to_sdr_builds_tone_mapping_graph() {
        let source = SourceColorState::new(ColorPrimaries::Bt2020, TransferFunction::Pq);
        let target = TargetColorState::sdr(ColorPrimaries::Bt709);
        let pipeline = VideoRenderPipeline::new(source, target);

        assert!(pipeline.requires_tone_mapping());
        assert!(pipeline.graph.contains(RenderPassKind::TransferDecode));
        assert!(pipeline.graph.contains(RenderPassKind::GamutMap));
        assert!(pipeline.graph.contains(RenderPassKind::ToneMap));
        assert!(pipeline.graph.contains(RenderPassKind::OutputTransform));
    }

    #[test]
    fn sdr_bt709_to_sdr_skips_tone_mapping() {
        let source = SourceColorState::new(ColorPrimaries::Bt709, TransferFunction::Srgb);
        let target = TargetColorState::sdr(ColorPrimaries::Bt709);
        let pipeline = VideoRenderPipeline::new(source, target);

        assert!(!pipeline.requires_tone_mapping());
        assert!(!pipeline.graph.contains(RenderPassKind::ToneMap));
    }

    #[test]
    fn matrix_defaults_follow_primaries() {
        let source = SourceColorState::new(ColorPrimaries::Bt2020, TransferFunction::Pq);
        let coeffs = source.matrix.luma_coefficients(source.primaries);

        assert!((coeffs.kr - 0.2627).abs() < 0.0001);
        assert!((coeffs.kg - 0.6780).abs() < 0.0001);
        assert!((coeffs.kb - 0.0593).abs() < 0.0001);
    }

    #[test]
    fn color_range_resolves_unspecified_to_fallback() {
        assert_eq!(
            ColorRange::Unspecified.resolve(ColorRange::Limited),
            ColorRange::Limited
        );
        assert_eq!(
            ColorRange::Full.resolve(ColorRange::Limited),
            ColorRange::Full
        );
    }
}
