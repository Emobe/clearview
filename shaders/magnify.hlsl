// Pixel shader — bilinear fullscreen magnification.
// The vertex shader emits a fullscreen triangle strip with UVs 0..1.

cbuffer Constants : register(b0)
{
    float2 center; // normalised mouse position (0..1)
    float  zoom;
    float  _pad;
};

Texture2D    screen_tex : register(t0);
SamplerState linear_smp : register(s0);

struct PSInput
{
    float4 pos : SV_Position;
    float2 uv  : TEXCOORD;
};

PSInput vs_main(uint id : SV_VertexID)
{
    // Generate a fullscreen quad from two triangles (triangle strip, 4 verts).
    // id: 0=(0,0), 1=(1,0), 2=(0,1), 3=(1,1)
    PSInput o;
    float2 uv  = float2((id & 1u) ? 1.0 : 0.0, (id >> 1u) ? 1.0 : 0.0);
    o.uv       = uv;
    o.pos      = float4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    return o;
}

float4 ps_main(PSInput i) : SV_Target
{
    // Map screen UV into zoomed sub-region centred on cursor.
    float2 sample_uv = (i.uv - 0.5) / zoom + center;
    // Clamp to edge to avoid border wrap artefacts.
    sample_uv = saturate(sample_uv);
    return screen_tex.Sample(linear_smp, sample_uv);
}
