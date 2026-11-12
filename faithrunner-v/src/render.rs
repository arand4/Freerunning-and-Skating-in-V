//! Draws Faith's arms and legs into GTA's frame, from ScriptHookV's present
//! callback (GTA's render thread, just before the frame is shown). The body has
//! its own depth buffer and draws over the world, as Mirror's Edge draws its
//! first-person body. Everything is recorded on a deferred context and run with
//! `ExecuteCommandList(.., restore = true)`, so GTA's own render state is untouched.
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use glam::Mat4;
use me_assets::Rgba;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use windows::core::{Interface, PCSTR, s};

use crate::log;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Vertex {
    pub p: [f32; 3],
    pub n: [f32; 3],
    pub uv: [f32; 2],
}

pub struct Section {
    pub indices: Vec<u32>,
    pub texture: Option<Rgba>,
}

/// What never changes: per part (arms, legs) its UVs and material sections.
pub struct PartStatic {
    pub uvs: Vec<[f32; 2]>,
    pub sections: Vec<Section>,
    pub legs: bool,
}

pub struct Scene {
    pub parts: Vec<PartStatic>,
}

/// One frame of one part: skinned vertices (camera-relative) and its view-projection.
pub struct PartFrame {
    pub verts: Vec<Vertex>,
    pub view_proj: Mat4,
}

type Published = (Arc<Scene>, Vec<PartFrame>);
static CURRENT: Mutex<Option<Published>> = Mutex::new(None);

/// Tests: the frames last published.
#[cfg(test)]
pub fn tests_take() -> Vec<PartFrame> {
    CURRENT.lock().unwrap().take().map(|(_, f)| f).unwrap_or_default()
}

/// Script thread: the body to draw from now on (None: nothing).
pub fn publish(p: Option<Published>) {
    *CURRENT.lock().unwrap() = p;
}

struct GpuPart {
    vb: ID3D11Buffer,
    capacity: usize,
    sections: Vec<(ID3D11Buffer, u32, ID3D11ShaderResourceView)>,
}

struct Gpu {
    device: ID3D11Device,
    immediate: ID3D11DeviceContext,
    deferred: ID3D11DeviceContext,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    sampler: ID3D11SamplerState,
    raster: ID3D11RasterizerState,
    depth_state: ID3D11DepthStencilState,
    cb: ID3D11Buffer,
    depth: Option<((u32, u32), ID3D11DepthStencilView)>,
    scene: Option<(*const Scene, Vec<GpuPart>)>,
}

thread_local! {
    static GPU: RefCell<Option<Gpu>> = const { RefCell::new(None) };
    static FAILED: RefCell<bool> = const { RefCell::new(false) };
}

const SHADER: &str = r#"
cbuffer C : register(b0) { float4x4 vp; float4 light; };
struct VI { float3 p : POSITION; float3 n : NORMAL; float2 uv : TEXCOORD0; };
struct VO { float4 p : SV_POSITION; float3 n : NORMAL; float2 uv : TEXCOORD0; };
VO vs(VI i) { VO o; o.p = mul(vp, float4(i.p, 1)); o.n = i.n; o.uv = i.uv; return o; }
Texture2D tex : register(t0);
SamplerState smp : register(s0);
float4 ps(VO i) : SV_TARGET {
    float3 c = tex.Sample(smp, i.uv).rgb;
    float d = saturate(dot(normalize(i.n), light.xyz));
    return float4(c * (light.w + 0.55 * d), 1);
}
"#;

fn compile(entry: PCSTR, target: PCSTR) -> windows::core::Result<ID3DBlob> {
    let mut code = None;
    let mut errors = None;
    let r = unsafe {
        D3DCompile(SHADER.as_ptr() as *const c_void, SHADER.len(), None, None, None, entry, target, 0, 0, &mut code, Some(&mut errors))
    };
    if let Err(e) = r {
        if let Some(b) = errors {
            let msg = unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) };
            log!("render: shader: {}", String::from_utf8_lossy(msg));
        }
        return Err(e);
    }
    Ok(code.unwrap())
}

fn blob(b: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }
}

impl Gpu {
    fn new(device: ID3D11Device) -> windows::core::Result<Gpu> {
        unsafe {
            let immediate = device.GetImmediateContext()?;
            let mut deferred = None;
            device.CreateDeferredContext(0, Some(&mut deferred))?;
            let vsb = compile(s!("vs"), s!("vs_5_0"))?;
            let psb = compile(s!("ps"), s!("ps_5_0"))?;
            let mut vs = None;
            device.CreateVertexShader(blob(&vsb), None, Some(&mut vs))?;
            let mut ps = None;
            device.CreatePixelShader(blob(&psb), None, Some(&mut ps))?;
            let el = |name: PCSTR, fmt, off| D3D11_INPUT_ELEMENT_DESC {
                SemanticName: name,
                SemanticIndex: 0,
                Format: fmt,
                InputSlot: 0,
                AlignedByteOffset: off,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            };
            let elems = [
                el(s!("POSITION"), DXGI_FORMAT_R32G32B32_FLOAT, 0),
                el(s!("NORMAL"), DXGI_FORMAT_R32G32B32_FLOAT, 12),
                el(s!("TEXCOORD"), DXGI_FORMAT_R32G32_FLOAT, 24),
            ];
            let mut layout = None;
            device.CreateInputLayout(&elems, blob(&vsb), Some(&mut layout))?;
            let mut sampler = None;
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_ANISOTROPIC,
                    AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
                    AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
                    AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
                    MaxAnisotropy: 8,
                    MaxLOD: f32::MAX,
                    ..Default::default()
                },
                Some(&mut sampler),
            )?;
            let mut raster = None;
            device.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC { FillMode: D3D11_FILL_SOLID, CullMode: D3D11_CULL_NONE, DepthClipEnable: true.into(), ..Default::default() },
                Some(&mut raster),
            )?;
            let mut depth_state = None;
            device.CreateDepthStencilState(
                &D3D11_DEPTH_STENCIL_DESC {
                    DepthEnable: true.into(),
                    DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
                    DepthFunc: D3D11_COMPARISON_LESS,
                    ..Default::default()
                },
                Some(&mut depth_state),
            )?;
            let mut cb = None;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC { ByteWidth: 80, Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32, ..Default::default() },
                None,
                Some(&mut cb),
            )?;
            Ok(Gpu {
                device,
                immediate,
                deferred: deferred.unwrap(),
                vs: vs.unwrap(),
                ps: ps.unwrap(),
                layout: layout.unwrap(),
                sampler: sampler.unwrap(),
                raster: raster.unwrap(),
                depth_state: depth_state.unwrap(),
                cb: cb.unwrap(),
                depth: None,
                scene: None,
            })
        }
    }

    fn texture(&self, t: Option<&Rgba>) -> windows::core::Result<ID3D11ShaderResourceView> {
        let grey = Rgba { width: 1, height: 1, pixels: vec![90, 90, 95, 255] };
        let t = t.filter(|t| t.pixels.len() >= (t.width * t.height * 4) as usize && t.width > 0).unwrap_or(&grey);
        unsafe {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: t.width,
                Height: t.height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_IMMUTABLE,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                ..Default::default()
            };
            let data = D3D11_SUBRESOURCE_DATA { pSysMem: t.pixels.as_ptr() as *const c_void, SysMemPitch: t.width * 4, SysMemSlicePitch: 0 };
            let mut tex = None;
            self.device.CreateTexture2D(&desc, Some(&data), Some(&mut tex))?;
            let mut srv = None;
            self.device.CreateShaderResourceView(&tex.unwrap(), None, Some(&mut srv))?;
            Ok(srv.unwrap())
        }
    }

    fn upload_scene(&mut self, scene: &Scene) -> windows::core::Result<Vec<GpuPart>> {
        let mut out = vec![];
        for part in &scene.parts {
            let mut sections = vec![];
            for sec in &part.sections {
                if sec.indices.is_empty() {
                    continue;
                }
                let mut ib = None;
                unsafe {
                    self.device.CreateBuffer(
                        &D3D11_BUFFER_DESC {
                            ByteWidth: (sec.indices.len() * 4) as u32,
                            Usage: D3D11_USAGE_IMMUTABLE,
                            BindFlags: D3D11_BIND_INDEX_BUFFER.0 as u32,
                            ..Default::default()
                        },
                        Some(&D3D11_SUBRESOURCE_DATA { pSysMem: sec.indices.as_ptr() as *const c_void, ..Default::default() }),
                        Some(&mut ib),
                    )?;
                }
                sections.push((ib.unwrap(), sec.indices.len() as u32, self.texture(sec.texture.as_ref())?));
            }
            let capacity = part.uvs.len().max(1);
            let vb = self.vertex_buffer(capacity)?;
            out.push(GpuPart { vb, capacity, sections });
        }
        Ok(out)
    }

    fn vertex_buffer(&self, n: usize) -> windows::core::Result<ID3D11Buffer> {
        let mut vb = None;
        unsafe {
            self.device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: (n * size_of::<Vertex>()) as u32,
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut vb),
            )?;
        }
        Ok(vb.unwrap())
    }

    fn draw(&mut self, swap: &IDXGISwapChain, scene: &Arc<Scene>, frames: &[PartFrame]) -> windows::core::Result<()> {
        let back: ID3D11Texture2D = unsafe { swap.GetBuffer(0)? };
        self.draw_into(&back, scene, frames)
    }

    fn draw_into(&mut self, back: &ID3D11Texture2D, scene: &Arc<Scene>, frames: &[PartFrame]) -> windows::core::Result<()> {
        unsafe {
            let mut bd = D3D11_TEXTURE2D_DESC::default();
            back.GetDesc(&mut bd);
            let rtv_desc = D3D11_RENDER_TARGET_VIEW_DESC {
                Format: match bd.Format {
                    DXGI_FORMAT_R8G8B8A8_TYPELESS => DXGI_FORMAT_R8G8B8A8_UNORM,
                    DXGI_FORMAT_B8G8R8A8_TYPELESS => DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_FORMAT_R10G10B10A2_TYPELESS => DXGI_FORMAT_R10G10B10A2_UNORM,
                    f => f,
                },
                ViewDimension: if bd.SampleDesc.Count > 1 { D3D11_RTV_DIMENSION_TEXTURE2DMS } else { D3D11_RTV_DIMENSION_TEXTURE2D },
                ..Default::default()
            };
            let mut rtv = None;
            self.device.CreateRenderTargetView(back, Some(&rtv_desc), Some(&mut rtv))?;
            let rtv = rtv.unwrap();
            let size = (bd.Width, bd.Height);
            if self.depth.as_ref().is_none_or(|(s, _)| *s != size) {
                let mut dt = None;
                self.device.CreateTexture2D(
                    &D3D11_TEXTURE2D_DESC {
                        Width: size.0,
                        Height: size.1,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_D24_UNORM_S8_UINT,
                        SampleDesc: bd.SampleDesc,
                        Usage: D3D11_USAGE_DEFAULT,
                        BindFlags: D3D11_BIND_DEPTH_STENCIL.0 as u32,
                        ..Default::default()
                    },
                    None,
                    Some(&mut dt),
                )?;
                let mut dsv = None;
                self.device.CreateDepthStencilView(&dt.unwrap(), None, Some(&mut dsv))?;
                self.depth = Some((size, dsv.unwrap()));
                log!("render: drawing into GTA's {}x{} frame (format {:?})", size.0, size.1, bd.Format);
            }
            let key = Arc::as_ptr(scene);
            if self.scene.as_ref().is_none_or(|(k, _)| *k != key) {
                let parts = self.upload_scene(scene)?;
                self.scene = Some((key, parts));
            }
            let dsv = self.depth.as_ref().unwrap().1.clone();
            let ctx = self.deferred.clone();
            ctx.OMSetRenderTargets(Some(&[Some(rtv)]), &dsv);
            ctx.ClearDepthStencilView(&dsv, D3D11_CLEAR_DEPTH.0 as u32, 1.0, 0);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT { Width: size.0 as f32, Height: size.1 as f32, MaxDepth: 1.0, ..Default::default() }]));
            ctx.RSSetState(&self.raster);
            ctx.OMSetDepthStencilState(&self.depth_state, 0);
            ctx.OMSetBlendState(None, None, 0xFFFF_FFFF);
            ctx.IASetInputLayout(&self.layout);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            let light = glam::Vec3::new(0.3, 1.0, 0.25).normalize();
            let parts = &mut self.scene.as_mut().unwrap().1;
            for (gp, f) in parts.iter_mut().zip(frames) {
                if f.verts.is_empty() {
                    continue;
                }
                if f.verts.len() > gp.capacity {
                    continue; // never: the skinned count is the mesh's
                }
                let mut cbdata = [0f32; 20];
                cbdata[..16].copy_from_slice(&f.view_proj.to_cols_array());
                cbdata[16..].copy_from_slice(&[light.x, light.y, light.z, 0.6]);
                ctx.UpdateSubresource(&self.cb, 0, None, cbdata.as_ptr() as *const c_void, 0, 0);
                ctx.UpdateSubresource(&gp.vb, 0, None, f.verts.as_ptr() as *const c_void, 0, 0);
                let stride = size_of::<Vertex>() as u32;
                let offset = 0u32;
                ctx.IASetVertexBuffers(0, 1, Some(&Some(gp.vb.clone())), Some(&stride), Some(&offset));
                for (ib, count, srv) in &gp.sections {
                    ctx.IASetIndexBuffer(ib, DXGI_FORMAT_R32_UINT, 0);
                    ctx.PSSetShaderResources(0, Some(&[Some(srv.clone())]));
                    ctx.DrawIndexed(*count, 0, 0);
                }
            }
            let mut list = None;
            ctx.FinishCommandList(false, Some(&mut list))?;
            self.immediate.ExecuteCommandList(&list.unwrap(), true);
        }
        Ok(())
    }
}

/// ScriptHookV's present callback (GTA's render thread).
pub extern "C" fn on_present(swap_chain: *mut c_void) {
    let _ = std::panic::catch_unwind(|| present(swap_chain));
}

fn present(swap_chain: *mut c_void) {
    if FAILED.with(|f| *f.borrow()) || swap_chain.is_null() {
        return;
    }
    let guard = CURRENT.lock().unwrap();
    let Some((scene, frames)) = guard.as_ref() else { return };
    let Some(swap) = (unsafe { IDXGISwapChain::from_raw_borrowed(&swap_chain) }) else { return };
    GPU.with(|g| {
        let mut g = g.borrow_mut();
        if g.is_none() {
            match unsafe { swap.GetDevice::<ID3D11Device>() }.and_then(Gpu::new) {
                Ok(gpu) => {
                    log!("render: ready");
                    *g = Some(gpu);
                }
                Err(e) => {
                    log!("render: couldn't set up drawing ({e}); Faith's body stays hidden");
                    FAILED.with(|f| *f.borrow_mut() = true);
                    return;
                }
            }
        }
        if let Err(e) = g.as_mut().unwrap().draw(swap, scene, frames) {
            log!("render: drawing failed ({e}); Faith's body stays hidden");
            FAILED.with(|f| *f.borrow_mut() = true);
        }
    });
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;

    /// Draws `frames` into a fresh 1280x720 picture over a sky colour and writes it as a BMP.
    pub fn render_to_bmp(scene: &Arc<Scene>, frames: &[PartFrame], path: &std::path::Path) -> (usize, usize) {
        unsafe {
            let mut device = None;
            let mut ctx = None;
            D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx))
                .expect("device");
            let device: ID3D11Device = device.unwrap();
            let ctx = ctx.unwrap();
            let (w, h) = (1280u32, 720u32);
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                ..Default::default()
            };
            let sky: Vec<u8> = (0..w * h).flat_map(|_| [150u8, 180, 210, 255]).collect();
            let mut back = None;
            device
                .CreateTexture2D(&desc, Some(&D3D11_SUBRESOURCE_DATA { pSysMem: sky.as_ptr() as *const c_void, SysMemPitch: w * 4, SysMemSlicePitch: 0 }), Some(&mut back))
                .unwrap();
            let back = back.unwrap();
            let mut gpu = Gpu::new(device.clone()).expect("gpu");
            gpu.draw_into(&back, scene, frames).expect("draw");
            let mut staging = None;
            device
                .CreateTexture2D(&D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, ..desc }, None, Some(&mut staging))
                .unwrap();
            let staging = staging.unwrap();
            ctx.CopyResource(&staging, &back);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m)).unwrap();
            let mut bmp = Vec::new();
            let row = (w * 3) as usize;
            let size = 54 + row * h as usize;
            bmp.extend(b"BM");
            bmp.extend((size as u32).to_le_bytes());
            bmp.extend([0u8; 4]);
            bmp.extend(54u32.to_le_bytes());
            bmp.extend(40u32.to_le_bytes());
            bmp.extend((w as i32).to_le_bytes());
            bmp.extend((h as i32).to_le_bytes());
            bmp.extend(1u16.to_le_bytes());
            bmp.extend(24u16.to_le_bytes());
            bmp.extend([0u8; 24]);
            let mut changed = 0usize;
            for y in (0..h).rev() {
                let src = std::slice::from_raw_parts((m.pData as *const u8).add((y * m.RowPitch) as usize), (w * 4) as usize);
                for px in src.chunks(4) {
                    if px[..3] != [150, 180, 210] {
                        changed += 1;
                    }
                    bmp.extend([px[2], px[1], px[0]]);
                }
            }
            ctx.Unmap(&staging, 0);
            std::fs::write(path, bmp).unwrap();
            (changed, (w * h) as usize)
        }
    }
}
