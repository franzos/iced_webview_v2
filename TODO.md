# TODO

## Stylo 0.16 / Blitz upgrade
Bump blitz pin to `79320c5` (Stylo 0.16). Blocked: Servo 0.1.0 is on Stylo 0.15 and Stylo declares `links = "servo_style_crate"`, so Cargo refuses two parallel versions even with `servo` feature inactive. Revisit when Servo publishes a Stylo 0.16 release.

## Blitz zero-copy GPU rendering
Eliminate the GPU→CPU→GPU readback in the Blitz engine. Blocked: iced 0.14 uses wgpu 27, anyrender_vello 0.8 uses wgpu 28. Revisit when iced ships a wgpu-28 release (master is on 0.15.0-dev / wgpu 29).
