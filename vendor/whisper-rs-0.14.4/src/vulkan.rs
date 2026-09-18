use std::ffi::CStr;
use whisper_rs_sys::{
    ggml_backend_buffer_type_t, ggml_backend_dev_backend_reg, ggml_backend_dev_buffer_type,
    ggml_backend_dev_count, ggml_backend_dev_description, ggml_backend_dev_get,
    ggml_backend_dev_memory, ggml_backend_reg_name,
};

#[derive(Debug, Clone)]
pub struct VKVram {
    pub free: usize,
    pub total: usize,
}

/// Human-readable device information
#[derive(Debug, Clone)]
pub struct VkDeviceInfo {
    pub id: i32,
    pub name: String,
    pub vram: VKVram,
    /// Buffer type to pass to `whisper::Backend::create_buffer`
    pub buf_type: ggml_backend_buffer_type_t,
}

// Łatka lokalna (diktate): oryginalny whisper-rs 0.14.4 woła
// ggml_backend_vk_get_device_count/description/memory/buffer_type, których
// wersja whisper.cpp dołączona do whisper-rs-sys 0.13.1 już nie eksportuje —
// ggml przeszło na ogólne API rejestru backendów (ggml_backend_dev_*,
// ggml_backend_reg_*), wspólne dla CPU/CUDA/Vulkan/... Ten plik odtwarza tę
// samą funkcjonalność (enumerację GPU Vulkan) przez to ogólne API, filtrując
// urządzenia po nazwie ich rejestru backendu ("Vulkan"). Publiczne API
// modułu (VkDeviceInfo, VKVram, list_devices) jest niezmienione.
pub fn list_devices() -> Vec<VkDeviceInfo> {
    unsafe {
        let n = ggml_backend_dev_count();
        let mut out = Vec::new();
        let mut vk_id: i32 = 0;
        for idx in 0..n {
            let dev = ggml_backend_dev_get(idx);
            if dev.is_null() {
                continue;
            }
            let reg = ggml_backend_dev_backend_reg(dev);
            if reg.is_null() {
                continue;
            }
            let reg_name_ptr = ggml_backend_reg_name(reg);
            let reg_name = if reg_name_ptr.is_null() {
                String::new()
            } else {
                CStr::from_ptr(reg_name_ptr).to_string_lossy().into_owned()
            };
            if reg_name != "Vulkan" {
                continue;
            }

            let desc_ptr = ggml_backend_dev_description(dev);
            let name = if desc_ptr.is_null() {
                String::new()
            } else {
                CStr::from_ptr(desc_ptr).to_string_lossy().into_owned()
            };

            let mut free = 0usize;
            let mut total = 0usize;
            ggml_backend_dev_memory(dev, &mut free, &mut total);

            out.push(VkDeviceInfo {
                id: vk_id,
                name,
                vram: VKVram { free, total },
                buf_type: ggml_backend_dev_buffer_type(dev),
            });
            vk_id += 1;
        }
        out
    }
}

#[cfg(test)]
mod vulkan_tests {
    use super::*;

    #[test]
    fn enumerate_must_not_panic() {
        let _ = list_devices();
    }

    #[test]
    fn sane_device_info() {
        let gpus = list_devices();
        let mut seen = std::collections::HashSet::new();

        for dev in &gpus {
            assert!(seen.insert(dev.id), "duplicated id {}", dev.id);
            assert!(!dev.name.trim().is_empty(), "GPU {} has empty name", dev.id);
            assert!(
                dev.vram.total >= dev.vram.free,
                "GPU {} total < free",
                dev.id
            );
        }
    }
}
