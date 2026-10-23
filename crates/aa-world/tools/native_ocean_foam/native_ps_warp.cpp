// Read-only evidence: executes the unmodified shipping cached WaterPS on CPU WARP.
// Synthetic 1x1 textures and constant interpolants isolate foam; no game process.
#include <windows.h>
#include <d3d11.h>
#include <d3dcompiler.h>
#include <wrl/client.h>
#include <array>
#include <cmath>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <vector>
using Microsoft::WRL::ComPtr;
void check(HRESULT hr, const char* what) {
    if (FAILED(hr)) { std::cerr << what << " failed 0x" << std::hex << hr << '\n'; std::exit(1); }
}
using V = std::array<float,4>;
int main(int argc, char** argv) {
    if (argc != 2) return 2;
    const auto dir = std::filesystem::path(argv[1]);
    ComPtr<ID3D11Device> device; ComPtr<ID3D11DeviceContext> ctx;
    D3D_FEATURE_LEVEL level;
    check(D3D11CreateDevice(nullptr,D3D_DRIVER_TYPE_WARP,nullptr,0,nullptr,0,D3D11_SDK_VERSION,&device,&level,&ctx),"CPU WARP device");
    std::ifstream source(dir/"native-ps-1baea2ad.dxbc",std::ios::binary);
    std::vector<char> bytes((std::istreambuf_iterator<char>(source)),{});
    ComPtr<ID3D11PixelShader> ps;
    check(device->CreatePixelShader(bytes.data(),bytes.size(),nullptr,&ps),"unmodified native WaterPS");
    const char* vsSource = R"(
cbuffer Inputs:register(b1) { float4 vals[8]; }
struct Out { float4 pos:SV_Position; float4 base:TEXCOORD0; float4 screen:TEXCOORD1;
float4 env:TEXCOORD2; float4 eye:TEXCOORD3; float4 gloss:TEXCOORD4;
float4 spec:TEXCOORD5; float4 world:TEXCOORD6; float4 normal:TEXCOORD7; };
Out main(uint id:SV_VertexID) { Out o; o.pos=float4(id==1?3:-1,id==2?3:-1,0,1);
o.base=vals[0];o.screen=vals[1];o.env=vals[2];o.eye=vals[3];o.gloss=vals[4];
o.spec=vals[5];o.world=vals[6];o.normal=vals[7];return o; })";
    ComPtr<ID3DBlob> vsCode,errors;
    auto hr=D3DCompile(vsSource,strlen(vsSource),nullptr,nullptr,nullptr,"main","vs_5_0",D3DCOMPILE_ENABLE_STRICTNESS,0,&vsCode,&errors);
    if(errors) std::cerr.write((char*)errors->GetBufferPointer(),errors->GetBufferSize());
    check(hr,"constant-interpolant VS");
    ComPtr<ID3D11VertexShader> vs;
    check(device->CreateVertexShader(vsCode->GetBufferPointer(),vsCode->GetBufferSize(),nullptr,&vs),"VS");
    D3D11_RASTERIZER_DESC raster={};raster.FillMode=D3D11_FILL_SOLID;raster.CullMode=D3D11_CULL_NONE;raster.DepthClipEnable=TRUE;
    ComPtr<ID3D11RasterizerState> rasterState;check(device->CreateRasterizerState(&raster,&rasterState),"raster");ctx->RSSetState(rasterState.Get());
    D3D11_VIEWPORT viewport={0,0,1,1,0,1};ctx->RSSetViewports(1,&viewport);
    D3D11_TEXTURE2D_DESC td={};td.Width=td.Height=td.MipLevels=td.ArraySize=td.SampleDesc.Count=1;
    td.Format=DXGI_FORMAT_R32G32B32A32_FLOAT;td.Usage=D3D11_USAGE_DEFAULT;td.BindFlags=D3D11_BIND_RENDER_TARGET;
    ComPtr<ID3D11Texture2D> target,staging;
    check(device->CreateTexture2D(&td,nullptr,&target),"target");
    ComPtr<ID3D11RenderTargetView> rtv;check(device->CreateRenderTargetView(target.Get(),nullptr,&rtv),"RTV");
    td.Usage=D3D11_USAGE_STAGING;td.BindFlags=0;td.CPUAccessFlags=D3D11_CPU_ACCESS_READ;
    check(device->CreateTexture2D(&td,nullptr,&staging),"staging");ctx->OMSetRenderTargets(1,rtv.GetAddressOf(),nullptr);
    std::array<ComPtr<ID3D11Buffer>,5> cbs;
    for(auto& cb:cbs) { D3D11_BUFFER_DESC d={};d.ByteWidth=256;d.Usage=D3D11_USAGE_DEFAULT;d.BindFlags=D3D11_BIND_CONSTANT_BUFFER;
        check(device->CreateBuffer(&d,nullptr,&cb),"constant buffer"); }
    ctx->VSSetConstantBuffers(1,1,cbs[1].GetAddressOf());
    for(int i:{0,3,4})ctx->PSSetConstantBuffers(i,1,cbs[i].GetAddressOf());
    std::array<ComPtr<ID3D11Texture2D>,6> textures;
    std::array<ComPtr<ID3D11ShaderResourceView>,6> srvs;
    for(int i=0;i<6;i++) { td.Usage=D3D11_USAGE_DEFAULT;td.BindFlags=D3D11_BIND_SHADER_RESOURCE;td.CPUAccessFlags=0;
        check(device->CreateTexture2D(&td,nullptr,&textures[i]),"input texture");
        check(device->CreateShaderResourceView(textures[i].Get(),nullptr,&srvs[i]),"input SRV");ctx->PSSetShaderResources(i,1,srvs[i].GetAddressOf()); }
    ComPtr<ID3D11SamplerState> sampler;
    D3D11_SAMPLER_DESC sd={};sd.Filter=D3D11_FILTER_MIN_MAG_MIP_LINEAR;sd.AddressU=sd.AddressV=sd.AddressW=D3D11_TEXTURE_ADDRESS_CLAMP;sd.MaxLOD=D3D11_FLOAT32_MAX;
    check(device->CreateSamplerState(&sd,&sampler),"sampler");for(int i=0;i<6;i++)ctx->PSSetSamplers(i,1,sampler.GetAddressOf());
    ctx->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);ctx->VSSetShader(vs.Get(),nullptr,0);ctx->PSSetShader(ps.Get(),nullptr,0);
    std::ofstream csv(dir/"native-ps-warp.csv");csv<<std::setprecision(10);
    csv<<"side,surface,under,foam,gloss,reflection,sun_r,sky_r,amount,delta_r,delta_g,delta_b,expected,max_error\n";
    float worst=0;int cases=0;
    for(float side:{-1.f,1.f})for(float surface:{.1f,.34f,.5f,1.f,8.f})for(float under:{0.f,.1f,.66f,2.f,100.f})
    for(float foam:{.25f,.75f,1.f})for(float gloss:{0.f,.5f,1.f})for(float reflection:{0.f,.25f,1.f}) {
        std::array<std::array<V,16>,5> constants={};
        constants[0][4][1]=1000;constants[0][6]={0,0,side,0};
        constants[1][0]={.4f,.2f,.1f,.3f};constants[1][1]={.5f*surface,.5f*surface,.85f,surface};
        constants[1][2]={.5f,.5f,0,1};constants[1][3]={0,0,side,side};constants[1][4]={32,16,0,0};constants[1][7]={0,0,1,0};
        constants[3][0]={.25f,.7f,1.2f,0};constants[3][1]={2,9,13,0};constants[3][2]={0,0,1,1};
        constants[4][3]={.7f,2,8,2};constants[4][4]={0,0,.02f,.05f};constants[4][5]={.05f,4,1,4};
        constants[4][6]={1,1.2f,.1f,.2f};constants[4][7]={-.045f,.079f,1.1f,0};constants[4][8]={.2f,1,0,12};
        std::array<V,6> tex={V{reflection,reflection,reflection,1},V{0,0,0,0},V{.5f,.5f,1,.5f},
            V{(surface+under)/1000,(surface+under)/1000,(surface+under)/1000,(surface+under)/1000},V{gloss,gloss,gloss,1},V{foam,foam,foam,1}};
        for(int i=0;i<6;i++)ctx->UpdateSubresource(textures[i].Get(),0,nullptr,tex[i].data(),16,0);
        V outputs[2];
        for(int enabled=0;enabled<2;enabled++) {
            constants[4][3][1]=enabled?2.f:0.f;
            for(int i:{0,1,3,4})ctx->UpdateSubresource(cbs[i].Get(),0,nullptr,constants[i].data(),0,0);
            ctx->Draw(3,0);ctx->CopyResource(staging.Get(),target.Get());D3D11_MAPPED_SUBRESOURCE mapped;
            check(ctx->Map(staging.Get(),0,D3D11_MAP_READ,0,&mapped),"readback");memcpy(outputs[enabled].data(),mapped.pData,16);ctx->Unmap(staging.Get(),0);
        }
        auto sat=[](float x){return fmaxf(0,fminf(1,x));};
        float fsi=sat(.7f*under)-.045f, g=.7f*gloss+.3f;
        float shape=foam*sat(foam-fsi*2*g)*sat(fsi-powf(sat(fsi-.079f),1.1f));
        float shadow=sat(.5f+powf(.999f*reflection,2));
        float expected=shape*(2*shadow+.25f)*sat(surface-.33f);
        float err=0;for(int c=0;c<3;c++)err=fmaxf(err,fabsf((outputs[1][c]-outputs[0][c])-expected));
        worst=fmaxf(worst,err);cases++;
        csv<<side<<','<<surface<<','<<under<<','<<foam<<','<<gloss<<','<<reflection<<",2,0.25,2";
        for(int c=0;c<3;c++)csv<<','<<(outputs[1][c]-outputs[0][c]);csv<<','<<expected<<','<<err<<'\n';
    }
    std::cout<<"Unmodified cached native WaterPS CPU WARP: "<<cases<<" cases, max absolute error "<<worst<<"\n";
    return worst<2e-6?0:1;
}
