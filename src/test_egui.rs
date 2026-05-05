fn test(device: &wgpu::Device, format: wgpu::TextureFormat) {
    let r = egui_wgpu::Renderer::new(device, format, None, 1);
}
