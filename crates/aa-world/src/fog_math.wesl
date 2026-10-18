// Exact analytic FogVolumeEllipsoidPS / FogVolumeBoxPS, with world-metre ray parameters.
fn opacity(local_from_world: mat4x4<f32>, center:vec3<f32>, density: f32, base: vec4<f32>, gradient: vec4<f32>, eye: vec3<f32>, direction: vec3<f32>, scene_distance: f32) -> f32 {
    if density == 0.0 { return 0.0; }
    let e=(local_from_world*vec4<f32>(eye-center,0.0)).xyz;
    let d=(local_from_world*vec4<f32>(direction,0.0)).xyz;
    var start=0.0; var end=0.0; var discr=0.0;
    if gradient.w > 0.5 {
        let len=length(d);let n=d/len;
        let a=(-vec3<f32>(1.0)-e)/n;let b=(vec3<f32>(1.0)-e)/n;
        let lo=clamp(min(a,b),vec3<f32>(-10000000.0),vec3<f32>(10000000.0));
        let hi=clamp(max(a,b),vec3<f32>(-10000000.0),vec3<f32>(10000000.0));
        start=max(0.0,max(lo.x,max(lo.y,lo.z)))/len;end=max(0.0,min(hi.x,min(hi.y,hi.z)))/len;
    } else {
        let len=length(d);let n=d/len;
        let b=2.0*dot(e,n);let c=4.0*(dot(e,e)-1.0);discr=b*b-c;
        if discr < 0.0 {return 0.0;}
        start=max(0.0,0.5*(-b-sqrt(discr)))/len;
        end=max(0.0,0.5*(-b+sqrt(discr)))/len;
    }
    end=max(start,min(scene_distance,end));
    let distance=end-start;
    if distance <= 0.0 {return 0.0;}
    let front=eye+direction*start;
    var integral=distance*exp(-dot(front-base.xyz,gradient.xyz));
    let height=dot(direction*distance,gradient.xyz);
    if abs(height)>0.001 {integral*=(1.0-exp(-height))/height;}
    if gradient.w < 0.5 {
        let soft=base.w*clamp((length(e)-0.95)*20.0,0.0,1.0);
        let arg=clamp(discr*soft+1.0-soft,0.0,1.0);
        integral*=arg*(2.0-arg);
    }
    return 1.0-exp2(-density*1.442695*integral);
}
