# Quby Render Queue 1.3 - capability / file probe
# Two modes:
#   --qrq-mode caps   : list everything this Blender build actually offers
#   --qrq-mode blend  : describe the .blend file that was opened
# Result is written as JSON to the path given by --qrq-out.

import sys
import json


def post_argv():
    a = list(sys.argv)
    if "--" in a:
        a = a[a.index("--") + 1:]
    return a


def argval(flag, default=None):
    a = post_argv()
    if flag in a:
        i = a.index(flag)
        if i + 1 < len(a):
            return a[i + 1]
    return default


def enum_ids(owner, attr):
    if owner is None:
        return []
    try:
        return [i.identifier for i in owner.bl_rna.properties[attr].enum_items]
    except Exception:
        pass
    try:
        return [i.identifier for i in owner.bl_rna.properties[attr].enum_items_static]
    except Exception:
        return []


def get(owner, attr, fallback=None):
    try:
        return getattr(owner, attr)
    except Exception:
        return fallback


def caps(bpy):
    scene = bpy.context.scene
    r = scene.render
    img = r.image_settings
    ff = get(r, "ffmpeg")
    vs = get(scene, "view_settings")
    ds = get(scene, "display_settings")

    colorspaces = []
    try:
        colorspaces = [i.identifier for i in
                       bpy.types.ColorManagedInputColorspaceSettings
                       .bl_rna.properties["name"].enum_items]
    except Exception:
        pass
    if not colorspaces:
        seq = get(scene, "sequencer_colorspace_settings")
        colorspaces = enum_ids(seq, "name")

    cycles_devices = []
    try:
        prefs = bpy.context.preferences.addons["cycles"].preferences
        cycles_devices = enum_ids(prefs, "compute_device_type")
    except Exception:
        pass

    denoisers = []
    cy = get(scene, "cycles")
    if cy is not None:
        denoisers = enum_ids(cy, "denoiser")

    return {
        "version": ".".join(str(x) for x in bpy.app.version),
        "version_string": bpy.app.version_string,
        "file_formats": enum_ids(img, "file_format"),
        "color_modes": enum_ids(img, "color_mode"),
        "color_depths": enum_ids(img, "color_depth"),
        "exr_codecs": enum_ids(img, "exr_codec"),
        "tiff_codecs": enum_ids(img, "tiff_codec"),
        "ff_containers": enum_ids(ff, "format"),
        "ff_video_codecs": enum_ids(ff, "codec"),
        "ff_audio_codecs": enum_ids(ff, "audio_codec"),
        "ff_crf": enum_ids(ff, "constant_rate_factor"),
        "ff_presets": enum_ids(ff, "ffmpeg_preset"),
        "engines": enum_ids(r, "engine"),
        "view_transforms": enum_ids(vs, "view_transform"),
        "looks": enum_ids(vs, "look"),
        "display_devices": enum_ids(ds, "display_device"),
        "colorspaces": colorspaces,
        "denoisers": denoisers,
        "cycles_device_types": cycles_devices,
        "has_time_remap": hasattr(r, "frame_map_old"),
    }


def blend_info(bpy):
    scenes = []
    for sc in bpy.data.scenes:
        r = sc.render
        base = get(r, "fps_base", 1.0) or 1.0
        try:
            fps = float(r.fps) / float(base)
        except Exception:
            fps = float(get(r, "fps", 24))
        seq_count = 0
        se = get(sc, "sequence_editor")
        if se is not None:
            for attr in ("strips_all", "sequences_all", "strips", "sequences"):
                coll = get(se, attr)
                if coll is not None:
                    try:
                        seq_count = len(coll)
                        break
                    except Exception:
                        pass
        node_count = 0
        nt = get(sc, "node_tree")
        if nt is not None:
            try:
                node_count = len(nt.nodes)
            except Exception:
                node_count = 0
        scenes.append({
            "name": sc.name,
            "frame_start": int(get(sc, "frame_start", 1)),
            "frame_end": int(get(sc, "frame_end", 250)),
            "frame_step": int(get(sc, "frame_step", 1)),
            "fps": round(fps, 4),
            "resolution_x": int(get(r, "resolution_x", 1920)),
            "resolution_y": int(get(r, "resolution_y", 1080)),
            "resolution_percentage": int(get(r, "resolution_percentage", 100)),
            "engine": str(get(r, "engine", "")),
            "file_format": str(get(r.image_settings, "file_format", "")),
            "filepath": str(get(r, "filepath", "")),
            "camera": sc.camera.name if sc.camera else "",
            "cameras": [o.name for o in bpy.data.objects if o.type == "CAMERA"],
            "view_layers": [vl.name for vl in sc.view_layers],
            "use_compositing": bool(get(r, "use_compositing", True)),
            "use_sequencer": bool(get(r, "use_sequencer", True)),
            "sequencer_strips": seq_count,
            "compositor_nodes": node_count,
            "use_nodes": bool(get(sc, "use_nodes", False)),
        })
    active = ""
    try:
        active = bpy.context.scene.name
    except Exception:
        pass
    return {
        "version": ".".join(str(x) for x in bpy.app.version),
        "active_scene": active,
        "scenes": scenes,
    }


def main():
    import bpy
    mode = argval("--qrq-mode", "caps")
    out = argval("--qrq-out")
    data = caps(bpy) if mode == "caps" else blend_info(bpy)
    payload = json.dumps(data)
    if out:
        with open(out, "w", encoding="utf-8") as f:
            f.write(payload)
    print("QRQPROBE_OK", flush=True)


try:
    main()
except Exception as exc:
    print("QRQPROBE_FAIL: %s" % exc, flush=True)
