# GPU Monitoring Sources

## Core rule

GPU utilization is not one universal Linux sysfs field. The value must be
reported only when the selected backend provides a meaningful counter.
Missing support is represented as `null` in JSON, not `0`.

## AMD

The reviewed Omarchy references and the AMDGPU Linux interface use DRM device
paths such as `/sys/class/drm/cardN/device/`. The useful read-only fields are:

```text
gpu_busy_percent
mem_info_vram_used
mem_info_vram_total
```

The GPU can also expose hwmon values under its device tree, including
`temp*_input`, `power*_average`, and frequency values. These are optional and
must be discovered rather than assumed.

`perfo` identifies AMD DRM cards by `device/vendor == 0x1002` and reads the
available AMD fields.

## Intel

Intel i915 and Xe do not guarantee an AMD-style device-wide
`gpu_busy_percent` file. Some kernels expose engine busy events through a
PMU, for example under `/sys/bus/event_source/devices/i915/events/`.

The current implementation probes i915 `*-busy` event definitions and uses
`perf_event_open` only when the PMU is available. This syscall is the fallback
for a metric that has no equivalent regular file; failure due to permissions,
kernel configuration, or missing events leaves utilization unavailable.

The implementation reports the highest engine utilization as the device
headline. This avoids summing parallel engines into an arbitrary value above
100% and makes a busy render or video engine visible.

## NVIDIA

NVIDIA does not provide a portable equivalent sysfs utilization field across
the supported driver stack. The MenuVitals reference uses optional
`nvidia-smi`; NVML is the more direct library API for utilization, memory,
temperature, power, and process data.

`perfo` does not yet ship an NVIDIA backend. Adding one requires an explicit
distribution decision: dynamically load NVML, add a build-time optional
integration, or use a bounded external query. It must not make NVIDIA a hard
runtime dependency for AMD/Intel users.

## Identity and multiple GPUs

DRM card numbers can change between boots, and connector entries also exist in
`/sys/class/drm`. Discovery must accept only names matching `card` followed by
digits and then inspect the device vendor/driver.

A machine can expose both an integrated and a discrete GPU. The JSON model is
therefore an array of devices rather than one global GPU scalar.

## JSON contract

The snapshot contains:

```json
{
  "gpu": {
    "devices": [
      {
        "name": "Intel GPU",
        "vendor": "Intel",
        "usage_percent": null,
        "memory_used_bytes": null,
        "memory_total_bytes": null,
        "temperature_c": null,
        "power_w": null
      }
    ]
  }
}
```

Every metric is optional because hardware and driver surfaces differ.
