use kuroko::renderer::pipeline::{
    ColorRange, MatrixCoefficients, SourceColorState, TargetColorState, VideoRenderPipeline,
};
use kuroko::{ColorPrimaries, TransferFunction};

fn main() {
    let source = SourceColorState::new(ColorPrimaries::Bt2020, TransferFunction::Pq)
        .range(ColorRange::Limited)
        .matrix(MatrixCoefficients::Bt2020NonConstantLuminance);
    let target = TargetColorState::sdr(ColorPrimaries::Bt709);
    let pipeline = VideoRenderPipeline::new(source, target);

    println!("Kuroko render pipeline plan");
    println!(
        "source: {:?} {:?}, range {:?}, matrix {:?}",
        pipeline.source.primaries,
        pipeline.source.transfer,
        pipeline.source.range,
        pipeline.source.matrix,
    );
    println!(
        "target: {:?} {:?}",
        pipeline.target.primaries, pipeline.target.transfer
    );
    println!("source transfer: {:?}", pipeline.source.transfer);
    println!("target transfer: {:?}", pipeline.target.transfer);
    println!("tone map: {:?}", pipeline.tone_map.operator);
    println!("scaler: {:?}", pipeline.scaler.kernel);
    println!(
        "requires tone mapping: {}",
        pipeline.requires_tone_mapping()
    );
    for (index, pass) in pipeline.graph.passes().iter().enumerate() {
        println!("  {:02}: {:?} - {}", index, pass.kind, pass.label);
    }
}
