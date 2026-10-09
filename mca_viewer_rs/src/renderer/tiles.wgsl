
@group(0) @binding(0) var samp: sampler;
@group(1) @binding(0) var tex: texture_2d<f32>;

struct Instance {
    // position/size in clip space
    @location(0) rect_min: vec2<f32>,
    @location(1) rect_size: vec2<f32>, // y negative
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: Instance) -> VOut {
    // Triangle strip corners: (0,0) (1,0) (0,1) (1,1)
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    var out: VOut;
    out.pos = vec4<f32>(inst.rect_min + corner * inst.rect_size, 0.0, 1.0);
    out.uv = corner; // scene is y-down, texture row 0 is top: no flip needed
    return out;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
