"""Run before -a: blender -b scene.blend --python-exit-code 1 --python queue_prepare.py -a.

Does not save the input .blend. Rebuilds caches after command-line overrides.
Also embedded in Quby Render Queue, where it runs after qrq_apply.py.
"""
import importlib.util
import os
from pathlib import Path
import sys
import bpy


def prepare():
    scene = bpy.context.scene
    tree = scene.compositing_node_group
    if not tree or not scene.render.use_compositing:
        return
    def contains_key(tree, seen):
        if tree.name in seen:
            return False
        seen.add(tree.name)
        for node in tree.nodes:
            child = getattr(node, 'node_tree', None)
            if child and (child.get('corridorkey_local') or contains_key(child, seen)):
                return True
        return False
    if not contains_key(tree, set()):
        return
    module = next((m for m in list(sys.modules.values())
                   if getattr(m, '__name__', '').split('.')[-1] in {'corridorkey_local', 'corridorkey_blender'}
                   and hasattr(m, 'prepare_scene')), None)
    if module is None:
        here = Path(__file__).parent
        local = Path(os.environ.get('LOCALAPPDATA', str(Path.home() / 'AppData/Local')))
        candidates = [here / '__init__.py', local / 'CorridorKey-Blender/Extension/corridorkey_local/__init__.py',
                      Path.home() / 'AppData/Local/CorridorKey-Blender/Extension/corridorkey_local/__init__.py']
        path = next((p for p in candidates if p.is_file() and (p.parent / 'core.py').is_file()), None)
        if path is None:
            raise RuntimeError('CorridorKey node found, but CorridorKey Local 0.2.0 is unavailable. Install the updated extension or its local integration package.')
        if hasattr(bpy.types.Node, 'ck_settings'):
            raise RuntimeError('An older CorridorKey extension is loaded. Update it to 0.2.0 before queue rendering.')
        spec = importlib.util.spec_from_file_location('corridorkey_local', path,
                                                     submodule_search_locations=[str(path.parent)])
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
        module.register()
    module.prepare_scene()


if __name__ == '__main__':
    prepare()
