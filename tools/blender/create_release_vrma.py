"""Create Yuukei's release VRMA motion set from the active VRM armature.

Run inside Blender after importing the default Yuukei VRM:

    exec(compile(open(r".../tools/blender/create_release_vrma.py", encoding="utf-8").read(),
                 "create_release_vrma.py", "exec"))

The script keeps movement in-place. DesktopStageManager owns screen movement;
only the hips bone receives small posture/bounce offsets.
"""

from __future__ import annotations

import json
import math
import os
from pathlib import Path

import bpy
from mathutils import Euler, Quaternion, Vector


PROJECT_ROOT = Path(r"C:\Users\minimarimo3\Sagyouba\yuukeiV2")
OUTPUT_DIR = PROJECT_ROOT / "packs" / "default-yuukei" / "motion"
PREVIEW_DIR = PROJECT_ROOT / "target" / "vrma-previews"
AUTHORING_BLEND = OUTPUT_DIR / "yuukei-release-motions.blend"
QA_REPORT = OUTPUT_DIR / "yuukei-release-motions.qa.json"

FPS = 30
ARMATURE_NAME = "Armature"

BONES = {
    "hips": "J_Bip_C_Hips",
    "spine": "J_Bip_C_Spine",
    "chest": "J_Bip_C_Chest",
    "upper_chest": "J_Bip_C_UpperChest",
    "neck": "J_Bip_C_Neck",
    "head": "J_Bip_C_Head",
    "left_shoulder": "J_Bip_L_Shoulder",
    "left_upper_arm": "J_Bip_L_UpperArm",
    "left_lower_arm": "J_Bip_L_LowerArm",
    "left_hand": "J_Bip_L_Hand",
    "right_shoulder": "J_Bip_R_Shoulder",
    "right_upper_arm": "J_Bip_R_UpperArm",
    "right_lower_arm": "J_Bip_R_LowerArm",
    "right_hand": "J_Bip_R_Hand",
    "left_upper_leg": "J_Bip_L_UpperLeg",
    "left_lower_leg": "J_Bip_L_LowerLeg",
    "left_foot": "J_Bip_L_Foot",
    "left_toes": "J_Bip_L_ToeBase",
    "right_upper_leg": "J_Bip_R_UpperLeg",
    "right_lower_leg": "J_Bip_R_LowerLeg",
    "right_foot": "J_Bip_R_Foot",
    "right_toes": "J_Bip_R_ToeBase",
}

# Conservative authoring limits. These are below healthy active-ROM maxima and
# are intended as animation QA limits, not medical claims.
QA_LIMITS_DEG = {
    "spine": (22, 15, 18),
    "chest": (22, 15, 18),
    "upper_chest": (25, 18, 22),
    "neck": (25, 20, 35),
    "head": (30, 25, 40),
    "left_upper_arm": (100, 105, 105),
    "right_upper_arm": (100, 105, 105),
    "left_lower_arm": (135, 25, 25),
    "right_lower_arm": (135, 25, 25),
    "left_upper_leg": (100, 45, 35),
    "right_upper_leg": (100, 45, 35),
    "left_lower_leg": (120, 15, 15),
    "right_lower_leg": (120, 15, 15),
    "left_foot": (35, 20, 20),
    "right_foot": (35, 20, 20),
}


def radians3(values: tuple[float, float, float]) -> tuple[float, float, float]:
    return tuple(math.radians(value) for value in values)


def merge_pose(*poses: dict) -> dict:
    result: dict = {}
    for pose in poses:
        for key, value in pose.items():
            if key in result and isinstance(result[key], dict) and isinstance(value, dict):
                result[key] = {**result[key], **value}
            else:
                result[key] = value
    return result


# Rest is a T-pose. All authored motions start from a relaxed anatomical pose:
# shoulders below 90 degrees of abduction, elbows unlocked, knees soft.
NEUTRAL = {
    "left_upper_arm": {"global": (0, 82, 0)},
    "right_upper_arm": {"global": (0, -82, 0)},
    "left_lower_arm": {"local": (3, 0, 0)},
    "right_lower_arm": {"local": (-3, 0, 0)},
    "left_hand": {"local": (0, 0, -4)},
    "right_hand": {"local": (0, 0, 4)},
    "left_upper_leg": {"global": (0, 0, 1.5)},
    "right_upper_leg": {"global": (0, 0, -1.5)},
}


def k(frame: int, pose: dict | None = None) -> tuple[int, dict]:
    return frame, merge_pose(NEUTRAL, pose or {})


MOTIONS = {
    "idle_breathe": {
        "loop": True,
        "frames": [
            k(1, {"hips": {"location": (0, 0, 0)}, "chest": {"global": (0, 0, -0.8)}, "head": {"global": (0, 0, 0.6)}}),
            k(31, {"hips": {"location": (0.004, 0, 0.004)}, "chest": {"global": (-1.2, 0, 0.8)}, "upper_chest": {"global": (-1.0, 0, 0)}, "head": {"global": (0.7, 0, -0.6)}}),
            k(61, {"hips": {"location": (0, 0, 0)}, "chest": {"global": (0, 0, -0.8)}, "head": {"global": (0, 0, 0.6)}}),
            k(91, {"hips": {"location": (-0.004, 0, 0.004)}, "chest": {"global": (-1.2, 0, 0.8)}, "upper_chest": {"global": (-1.0, 0, 0)}, "head": {"global": (0.7, 0, -0.6)}}),
            k(121, {"hips": {"location": (0, 0, 0)}, "chest": {"global": (0, 0, -0.8)}, "head": {"global": (0, 0, 0.6)}}),
        ],
    },
    "idle_look_around": {
        "loop": False,
        "frames": [
            k(1),
            k(16, {"upper_chest": {"global": (0, 0, -3)}, "neck": {"global": (0, 0, -8)}, "head": {"global": (-2, 0, -16)}}),
            k(38, {"upper_chest": {"global": (0, 0, -3)}, "neck": {"global": (0, 0, -8)}, "head": {"global": (-2, 0, -16)}}),
            k(55),
            k(70, {"upper_chest": {"global": (0, 0, 2)}, "neck": {"global": (0, 0, 6)}, "head": {"global": (1, 0, 12)}}),
            k(86),
        ],
    },
    "idle_weight_shift": {
        "loop": True,
        "frames": [
            k(1, {"hips": {"location": (-0.018, 0, 0)}, "spine": {"global": (0, -2.5, 0)}, "left_lower_leg": {"local": (3, 0, 0)}}),
            k(31, {"hips": {"location": (0, 0, 0.006)}, "spine": {"global": (0, 0, 0)}}),
            k(61, {"hips": {"location": (0.018, 0, 0)}, "spine": {"global": (0, 2.5, 0)}, "right_lower_leg": {"local": (-3, 0, 0)}}),
            k(91, {"hips": {"location": (0, 0, 0.006)}, "spine": {"global": (0, 0, 0)}}),
            k(121, {"hips": {"location": (-0.018, 0, 0)}, "spine": {"global": (0, -2.5, 0)}, "left_lower_leg": {"local": (3, 0, 0)}}),
        ],
    },
    "walk": {
        "loop": True,
        "frames": [
            k(1, {
                "hips": {"location": (0, 0, 0)},
                "hips_rot": {"global": (0, 0, -3)},
                "left_upper_leg": {"global": (-22, 0, 1.5)},
                "right_upper_leg": {"global": (22, 0, -1.5)},
                "left_lower_leg": {"local": (-8, 0, 0)},
                "right_lower_leg": {"local": (-28, 0, 0)},
                "left_foot": {"local": (8, 0, 0)},
                "right_foot": {"local": (-8, 0, 0)},
                "left_upper_arm": {"global": (17, 82, 0)},
                "right_upper_arm": {"global": (-17, -82, 0)},
                "upper_chest": {"global": (2, 0, 3)},
            }),
            k(7, {
                "hips": {"location": (0.008, 0, 0.018)},
                "left_upper_leg": {"global": (-5, 0, 1.5)},
                "right_upper_leg": {"global": (5, 0, -1.5)},
                "left_lower_leg": {"local": (-30, 0, 0)},
                "right_lower_leg": {"local": (-8, 0, 0)},
                "left_foot": {"local": (2, 0, 0)},
                "right_foot": {"local": (8, 0, 0)},
                "left_upper_arm": {"global": (5, 82, 0)},
                "right_upper_arm": {"global": (-5, -82, 0)},
            }),
            k(13, {
                "hips": {"location": (0, 0, 0)},
                "hips_rot": {"global": (0, 0, 3)},
                "left_upper_leg": {"global": (22, 0, 1.5)},
                "right_upper_leg": {"global": (-22, 0, -1.5)},
                "left_lower_leg": {"local": (-28, 0, 0)},
                "right_lower_leg": {"local": (-8, 0, 0)},
                "left_foot": {"local": (-8, 0, 0)},
                "right_foot": {"local": (8, 0, 0)},
                "left_upper_arm": {"global": (-17, 82, 0)},
                "right_upper_arm": {"global": (17, -82, 0)},
                "upper_chest": {"global": (2, 0, -3)},
            }),
            k(19, {
                "hips": {"location": (-0.008, 0, 0.018)},
                "left_upper_leg": {"global": (5, 0, 1.5)},
                "right_upper_leg": {"global": (-5, 0, -1.5)},
                "left_lower_leg": {"local": (-8, 0, 0)},
                "right_lower_leg": {"local": (-30, 0, 0)},
                "left_foot": {"local": (8, 0, 0)},
                "right_foot": {"local": (2, 0, 0)},
                "left_upper_arm": {"global": (-5, 82, 0)},
                "right_upper_arm": {"global": (5, -82, 0)},
            }),
            k(25, {
                "hips": {"location": (0, 0, 0)},
                "hips_rot": {"global": (0, 0, -3)},
                "left_upper_leg": {"global": (-22, 0, 1.5)},
                "right_upper_leg": {"global": (22, 0, -1.5)},
                "left_lower_leg": {"local": (-8, 0, 0)},
                "right_lower_leg": {"local": (-28, 0, 0)},
                "left_foot": {"local": (8, 0, 0)},
                "right_foot": {"local": (-8, 0, 0)},
                "left_upper_arm": {"global": (17, 82, 0)},
                "right_upper_arm": {"global": (-17, -82, 0)},
                "upper_chest": {"global": (2, 0, 3)},
            }),
        ],
    },
    "grab_start": {
        "loop": False,
        "frames": [
            k(1),
            k(5, {
                "hips": {"location": (0, 0, 0.025)},
                "spine": {"global": (-8, 0, 0)},
                "upper_chest": {"global": (-9, 0, 0)},
                "head": {"global": (8, 0, 0)},
                "left_upper_arm": {"global": (-20, 62, -5)},
                "right_upper_arm": {"global": (-20, -62, 5)},
                "left_lower_arm": {"local": (-22, 0, 0)},
                "right_lower_arm": {"local": (22, 0, 0)},
                "left_upper_leg": {"global": (-12, -4, 2)},
                "right_upper_leg": {"global": (-18, 4, -2)},
                "left_lower_leg": {"local": (-22, 0, 0)},
                "right_lower_leg": {"local": (-30, 0, 0)},
            }),
            k(14, {
                "hips": {"location": (0, 0, 0.04)},
                "spine": {"global": (-5, 0, 0)},
                "upper_chest": {"global": (-6, 0, 0)},
                "head": {"global": (5, 0, 0)},
                "left_upper_arm": {"global": (-12, 72, -4)},
                "right_upper_arm": {"global": (-15, -68, 4)},
                "left_lower_arm": {"local": (-15, 0, 0)},
                "right_lower_arm": {"local": (18, 0, 0)},
                "left_upper_leg": {"global": (-10, -3, 2)},
                "right_upper_leg": {"global": (-15, 3, -2)},
                "left_lower_leg": {"local": (-20, 0, 0)},
                "right_lower_leg": {"local": (-26, 0, 0)},
            }),
        ],
    },
    "grab_hold": {
        "loop": True,
        "frames": [
            k(1, {
                "hips": {"location": (0, 0, 0.03)},
                "spine": {"global": (-5, 0, -2)},
                "head": {"global": (5, 0, 4)},
                "left_upper_arm": {"global": (-12, 72, -4)},
                "right_upper_arm": {"global": (-15, -68, 4)},
                "left_lower_arm": {"local": (-15, 0, 0)},
                "right_lower_arm": {"local": (18, 0, 0)},
                "left_upper_leg": {"global": (-10, -3, 2)},
                "right_upper_leg": {"global": (-15, 3, -2)},
                "left_lower_leg": {"local": (-20, 0, 0)},
                "right_lower_leg": {"local": (-26, 0, 0)},
            }),
            k(16, {
                "hips": {"location": (0.008, 0, 0.02)},
                "spine": {"global": (-4, 0, 2)},
                "head": {"global": (4, 0, -3)},
                "left_upper_arm": {"global": (-15, 70, 2)},
                "right_upper_arm": {"global": (-10, -72, -2)},
                "left_lower_arm": {"local": (-20, 0, 0)},
                "right_lower_arm": {"local": (14, 0, 0)},
                "left_upper_leg": {"global": (-15, -2, 2)},
                "right_upper_leg": {"global": (-10, 2, -2)},
                "left_lower_leg": {"local": (-26, 0, 0)},
                "right_lower_leg": {"local": (-20, 0, 0)},
            }),
            k(31, {
                "hips": {"location": (0, 0, 0.03)},
                "spine": {"global": (-5, 0, -2)},
                "head": {"global": (5, 0, 4)},
                "left_upper_arm": {"global": (-12, 72, -4)},
                "right_upper_arm": {"global": (-15, -68, 4)},
                "left_lower_arm": {"local": (-15, 0, 0)},
                "right_lower_arm": {"local": (18, 0, 0)},
                "left_upper_leg": {"global": (-10, -3, 2)},
                "right_upper_leg": {"global": (-15, 3, -2)},
                "left_lower_leg": {"local": (-20, 0, 0)},
                "right_lower_leg": {"local": (-26, 0, 0)},
            }),
        ],
    },
    "drop_land": {
        "loop": False,
        "frames": [
            k(1, {
                "hips": {"location": (0, 0, 0.04)},
                "left_upper_leg": {"global": (-12, -2, 2)},
                "right_upper_leg": {"global": (-15, 2, -2)},
                "left_lower_leg": {"local": (-24, 0, 0)},
                "right_lower_leg": {"local": (-28, 0, 0)},
            }),
            k(5, {
                "hips": {"location": (0, 0, -0.12)},
                "spine": {"global": (12, 0, 0)},
                "chest": {"global": (7, 0, 0)},
                "head": {"global": (-8, 0, 0)},
                "left_upper_leg": {"global": (-34, -3, 2)},
                "right_upper_leg": {"global": (-34, 3, -2)},
                "left_lower_leg": {"local": (-42, 0, 0)},
                "right_lower_leg": {"local": (-42, 0, 0)},
                "left_foot": {"local": (12, 0, 0)},
                "right_foot": {"local": (12, 0, 0)},
                "left_upper_arm": {"global": (-28, 62, -5)},
                "right_upper_arm": {"global": (-28, -62, 5)},
                "left_lower_arm": {"local": (-12, 0, 0)},
                "right_lower_arm": {"local": (12, 0, 0)},
            }),
            k(13, {
                "hips": {"location": (0, 0, -0.045)},
                "spine": {"global": (5, 0, 0)},
                "head": {"global": (-3, 0, 0)},
                "left_upper_leg": {"global": (-15, -1, 2)},
                "right_upper_leg": {"global": (-15, 1, -2)},
                "left_lower_leg": {"local": (-18, 0, 0)},
                "right_lower_leg": {"local": (-18, 0, 0)},
                "left_foot": {"local": (5, 0, 0)},
                "right_foot": {"local": (5, 0, 0)},
            }),
            k(24),
        ],
    },
    "drop_recover": {
        "loop": False,
        "frames": [
            k(1, {
                "hips": {"location": (0, 0, -0.05)},
                "spine": {"global": (5, 0, 0)},
                "left_upper_leg": {"global": (-16, 0, 2)},
                "right_upper_leg": {"global": (-16, 0, -2)},
                "left_lower_leg": {"local": (-20, 0, 0)},
                "right_lower_leg": {"local": (-20, 0, 0)},
            }),
            k(14, {"hips": {"location": (0, 0, 0.012)}, "upper_chest": {"global": (-3, 0, 0)}, "head": {"global": (-2, 0, 0)}}),
            k(26),
        ],
    },
    "perch_sit_down": {
        "loop": False,
        "frames": [
            k(1),
            k(12, {
                "hips": {"location": (0, 0.015, -0.14)},
                "spine": {"global": (7, 0, 0)},
                "left_upper_leg": {"global": (-48, -4, 4)},
                "right_upper_leg": {"global": (-48, 4, -4)},
                "left_lower_leg": {"local": (-62, 0, 0)},
                "right_lower_leg": {"local": (-62, 0, 0)},
                "left_foot": {"local": (-5, 0, 0)},
                "right_foot": {"local": (-5, 0, 0)},
                "left_upper_arm": {"global": (5, 72, 0)},
                "right_upper_arm": {"global": (5, -72, 0)},
            }),
            k(26, {
                "hips": {"location": (0, 0.03, -0.22)},
                "spine": {"global": (4, 0, 0)},
                "left_upper_leg": {"global": (-72, -5, 5)},
                "right_upper_leg": {"global": (-72, 5, -5)},
                "left_lower_leg": {"local": (-88, 0, 0)},
                "right_lower_leg": {"local": (-88, 0, 0)},
                "left_foot": {"local": (-8, 0, 0)},
                "right_foot": {"local": (-8, 0, 0)},
                "left_upper_arm": {"global": (8, 72, 0)},
                "right_upper_arm": {"global": (8, -72, 0)},
                "left_lower_arm": {"local": (-12, 0, 0)},
                "right_lower_arm": {"local": (12, 0, 0)},
            }),
        ],
    },
    "perch_idle": {
        "loop": True,
        "frames": [
            k(1, {
                "hips": {"location": (0, 0.03, -0.22)},
                "spine": {"global": (4, 0, -1)},
                "left_upper_leg": {"global": (-72, -5, 5)},
                "right_upper_leg": {"global": (-72, 5, -5)},
                "left_lower_leg": {"local": (-84, 0, 0)},
                "right_lower_leg": {"local": (-92, 0, 0)},
                "left_foot": {"local": (-8, 0, 0)},
                "right_foot": {"local": (-12, 0, 0)},
                "left_upper_arm": {"global": (8, 72, 0)},
                "right_upper_arm": {"global": (8, -72, 0)},
                "left_lower_arm": {"local": (-12, 0, 0)},
                "right_lower_arm": {"local": (12, 0, 0)},
            }),
            k(31, {
                "hips": {"location": (0.006, 0.03, -0.216)},
                "spine": {"global": (3, 0, 1)},
                "head": {"global": (0, 0, -2)},
                "left_upper_leg": {"global": (-72, -5, 5)},
                "right_upper_leg": {"global": (-72, 5, -5)},
                "left_lower_leg": {"local": (-94, 0, 0)},
                "right_lower_leg": {"local": (-82, 0, 0)},
                "left_foot": {"local": (-13, 0, 0)},
                "right_foot": {"local": (-7, 0, 0)},
                "left_upper_arm": {"global": (8, 72, 0)},
                "right_upper_arm": {"global": (8, -72, 0)},
                "left_lower_arm": {"local": (-12, 0, 0)},
                "right_lower_arm": {"local": (12, 0, 0)},
            }),
            k(61, {
                "hips": {"location": (0, 0.03, -0.22)},
                "spine": {"global": (4, 0, -1)},
                "left_upper_leg": {"global": (-72, -5, 5)},
                "right_upper_leg": {"global": (-72, 5, -5)},
                "left_lower_leg": {"local": (-84, 0, 0)},
                "right_lower_leg": {"local": (-92, 0, 0)},
                "left_foot": {"local": (-8, 0, 0)},
                "right_foot": {"local": (-12, 0, 0)},
                "left_upper_arm": {"global": (8, 72, 0)},
                "right_upper_arm": {"global": (8, -72, 0)},
                "left_lower_arm": {"local": (-12, 0, 0)},
                "right_lower_arm": {"local": (12, 0, 0)},
            }),
        ],
    },
    "perch_stand_up": {
        "loop": False,
        "frames": [
            k(1, {
                "hips": {"location": (0, 0.03, -0.22)},
                "spine": {"global": (4, 0, 0)},
                "left_upper_leg": {"global": (-72, -5, 5)},
                "right_upper_leg": {"global": (-72, 5, -5)},
                "left_lower_leg": {"local": (-88, 0, 0)},
                "right_lower_leg": {"local": (-88, 0, 0)},
                "left_foot": {"local": (-8, 0, 0)},
                "right_foot": {"local": (-8, 0, 0)},
            }),
            k(14, {
                "hips": {"location": (0, 0.015, -0.10)},
                "spine": {"global": (8, 0, 0)},
                "left_upper_leg": {"global": (-36, -2, 3)},
                "right_upper_leg": {"global": (-36, 2, -3)},
                "left_lower_leg": {"local": (-42, 0, 0)},
                "right_lower_leg": {"local": (-42, 0, 0)},
            }),
            k(28),
        ],
    },
    "wave": {
        "loop": False,
        "frames": [
            k(1),
            k(10, {
                "right_upper_arm": {"global": (-18, -28, 8), "local": (0, 90, 0)},
                "right_lower_arm": {"local": (95, 0, 0)},
                "right_hand": {"local": (0, 0, 8)},
                "upper_chest": {"global": (0, 0, -3)},
                "head": {"global": (0, 0, -4)},
            }),
            k(20, {
                "right_upper_arm": {"global": (-18, -28, -4), "local": (0, 90, 0)},
                "right_lower_arm": {"local": (98, 0, 0)},
                "right_hand": {"local": (0, 0, -15)},
                "upper_chest": {"global": (0, 0, -3)},
                "head": {"global": (0, 0, -4)},
            }),
            k(30, {
                "right_upper_arm": {"global": (-18, -28, 8), "local": (0, 90, 0)},
                "right_lower_arm": {"local": (95, 0, 0)},
                "right_hand": {"local": (0, 0, 15)},
                "upper_chest": {"global": (0, 0, -3)},
                "head": {"global": (0, 0, -4)},
            }),
            k(40, {
                "right_upper_arm": {"global": (-18, -28, -4), "local": (0, 90, 0)},
                "right_lower_arm": {"local": (98, 0, 0)},
                "right_hand": {"local": (0, 0, -15)},
                "upper_chest": {"global": (0, 0, -3)},
                "head": {"global": (0, 0, -4)},
            }),
            k(50, {
                "right_upper_arm": {"global": (-18, -28, 8), "local": (0, 90, 0)},
                "right_lower_arm": {"local": (95, 0, 0)},
                "right_hand": {"local": (0, 0, 10)},
                "upper_chest": {"global": (0, 0, -3)},
                "head": {"global": (0, 0, -4)},
            }),
            k(66),
        ],
    },
    "poke_react": {
        "loop": False,
        "frames": [
            k(1),
            k(5, {
                "hips": {"location": (0, 0.018, 0.005)},
                "spine": {"global": (-9, 0, 0)},
                "chest": {"global": (-7, 0, 0)},
                "head": {"global": (10, 0, 0)},
                "left_upper_arm": {"global": (-10, 70, -2)},
                "right_upper_arm": {"global": (-10, -70, 2)},
            }),
            k(12, {"hips": {"location": (0, 0.006, 0)}, "spine": {"global": (-3, 0, 0)}, "head": {"global": (3, 0, 0)}}),
            k(24),
        ],
    },
    "pat_react": {
        "loop": False,
        "frames": [
            k(1),
            k(12, {
                "hips": {"location": (0.008, 0, 0)},
                "upper_chest": {"global": (2, -2, 0)},
                "neck": {"global": (5, -4, 0)},
                "head": {"global": (9, -7, 0)},
                "left_upper_arm": {"global": (2, 86, 0)},
                "right_upper_arm": {"global": (2, -86, 0)},
            }),
            k(30, {
                "hips": {"location": (0.008, 0, 0)},
                "upper_chest": {"global": (2, -2, 0)},
                "neck": {"global": (5, -4, 0)},
                "head": {"global": (9, -7, 0)},
                "left_upper_arm": {"global": (2, 86, 0)},
                "right_upper_arm": {"global": (2, -86, 0)},
            }),
            k(46),
        ],
    },
    "sleep_enter": {
        "loop": False,
        "frames": [
            k(1),
            k(25, {
                "hips": {"location": (0, 0.012, -0.05)},
                "spine": {"global": (9, 0, 0)},
                "chest": {"global": (8, 0, 0)},
                "upper_chest": {"global": (6, 0, 0)},
                "neck": {"global": (6, 0, 0)},
                "head": {"global": (12, -5, 0)},
                "left_upper_leg": {"global": (15, -2, 2)},
                "right_upper_leg": {"global": (15, 2, -2)},
                "left_lower_leg": {"local": (-18, 0, 0)},
                "right_lower_leg": {"local": (-18, 0, 0)},
                "left_upper_arm": {"global": (8, 88, 0)},
                "right_upper_arm": {"global": (8, -88, 0)},
            }),
            k(60, {
                "hips": {"location": (0.012, 0.018, -0.075)},
                "spine": {"global": (12, -3, 0)},
                "chest": {"global": (10, -3, 0)},
                "upper_chest": {"global": (7, -2, 0)},
                "neck": {"global": (8, -4, 0)},
                "head": {"global": (15, -10, 0)},
                "left_upper_leg": {"global": (22, -3, 3)},
                "right_upper_leg": {"global": (18, 3, -3)},
                "left_lower_leg": {"local": (-28, 0, 0)},
                "right_lower_leg": {"local": (-22, 0, 0)},
                "left_upper_arm": {"global": (10, 90, 0)},
                "right_upper_arm": {"global": (10, -90, 0)},
            }),
        ],
    },
    "sleep_loop": {
        "loop": True,
        "frames": [
            k(1, {
                "hips": {"location": (0.012, 0.018, -0.075)},
                "spine": {"global": (12, -3, 0)},
                "chest": {"global": (10, -3, 0)},
                "upper_chest": {"global": (7, -2, 0)},
                "neck": {"global": (8, -4, 0)},
                "head": {"global": (15, -10, 0)},
                "left_upper_leg": {"global": (22, -3, 3)},
                "right_upper_leg": {"global": (18, 3, -3)},
                "left_lower_leg": {"local": (-28, 0, 0)},
                "right_lower_leg": {"local": (-22, 0, 0)},
                "left_upper_arm": {"global": (10, 90, 0)},
                "right_upper_arm": {"global": (10, -90, 0)},
            }),
            k(46, {
                "hips": {"location": (0.012, 0.018, -0.071)},
                "spine": {"global": (11, -3, 0)},
                "chest": {"global": (8.5, -3, 0)},
                "upper_chest": {"global": (5.5, -2, 0)},
                "neck": {"global": (8, -4, 0)},
                "head": {"global": (15, -10, 0)},
                "left_upper_leg": {"global": (22, -3, 3)},
                "right_upper_leg": {"global": (18, 3, -3)},
                "left_lower_leg": {"local": (-28, 0, 0)},
                "right_lower_leg": {"local": (-22, 0, 0)},
                "left_upper_arm": {"global": (10, 90, 0)},
                "right_upper_arm": {"global": (10, -90, 0)},
            }),
            k(91, {
                "hips": {"location": (0.012, 0.018, -0.075)},
                "spine": {"global": (12, -3, 0)},
                "chest": {"global": (10, -3, 0)},
                "upper_chest": {"global": (7, -2, 0)},
                "neck": {"global": (8, -4, 0)},
                "head": {"global": (15, -10, 0)},
                "left_upper_leg": {"global": (22, -3, 3)},
                "right_upper_leg": {"global": (18, 3, -3)},
                "left_lower_leg": {"local": (-28, 0, 0)},
                "right_lower_leg": {"local": (-22, 0, 0)},
                "left_upper_arm": {"global": (10, 90, 0)},
                "right_upper_arm": {"global": (10, -90, 0)},
            }),
        ],
    },
    "wake_up": {
        "loop": False,
        "frames": [
            k(1, {
                "hips": {"location": (0.012, 0.018, -0.075)},
                "spine": {"global": (12, -3, 0)},
                "chest": {"global": (10, -3, 0)},
                "upper_chest": {"global": (7, -2, 0)},
                "neck": {"global": (8, -4, 0)},
                "head": {"global": (15, -10, 0)},
                "left_upper_leg": {"global": (22, -3, 3)},
                "right_upper_leg": {"global": (18, 3, -3)},
                "left_lower_leg": {"local": (-28, 0, 0)},
                "right_lower_leg": {"local": (-22, 0, 0)},
            }),
            k(20, {
                "hips": {"location": (0, 0, -0.025)},
                "spine": {"global": (4, 0, 0)},
                "head": {"global": (8, 0, 0)},
                "left_upper_leg": {"global": (8, 0, 2)},
                "right_upper_leg": {"global": (8, 0, -2)},
                "left_lower_leg": {"local": (-10, 0, 0)},
                "right_lower_leg": {"local": (-10, 0, 0)},
            }),
            k(38, {
                "hips": {"location": (0, 0, 0.018)},
                "spine": {"global": (-5, 0, 0)},
                "upper_chest": {"global": (-7, 0, 0)},
                "head": {"global": (-5, 0, 0)},
                "left_upper_arm": {"global": (-15, 42, 0)},
                "right_upper_arm": {"global": (-15, -42, 0)},
                "left_lower_arm": {"local": (-8, 0, 0)},
                "right_lower_arm": {"local": (8, 0, 0)},
            }),
            k(58),
        ],
    },
    "talk_small": {
        "loop": True,
        "frames": [
            k(1, {
                "upper_chest": {"global": (0, 0, -1.5)},
                "right_upper_arm": {"global": (-14, -68, 2)},
                "right_lower_arm": {"local": (38, 0, 0)},
                "right_hand": {"local": (0, 0, 8)},
                "head": {"global": (0, 0, -2)},
            }),
            k(13, {
                "upper_chest": {"global": (-2, 0, 1.5)},
                "right_upper_arm": {"global": (-22, -62, -3)},
                "right_lower_arm": {"local": (48, 0, 0)},
                "right_hand": {"local": (0, 0, -5)},
                "head": {"global": (-2, 0, 2)},
            }),
            k(25, {
                "upper_chest": {"global": (0, 0, -1.5)},
                "right_upper_arm": {"global": (-14, -68, 2)},
                "right_lower_arm": {"local": (38, 0, 0)},
                "right_hand": {"local": (0, 0, 8)},
                "head": {"global": (0, 0, -2)},
            }),
        ],
    },
}


def armature() -> bpy.types.Object:
    obj = bpy.data.objects.get(ARMATURE_NAME)
    if obj is None or obj.type != "ARMATURE":
        raise RuntimeError(f"VRM armature {ARMATURE_NAME!r} was not found")
    missing = [name for name in BONES.values() if name not in obj.pose.bones]
    if missing:
        raise RuntimeError(f"Missing humanoid bones: {missing}")
    return obj


def clear_pose(arm: bpy.types.Object) -> None:
    for pose_bone in arm.pose.bones:
        pose_bone.rotation_mode = "QUATERNION"
        pose_bone.rotation_quaternion = Quaternion((1, 0, 0, 0))
        pose_bone.location = Vector((0, 0, 0))
        pose_bone.scale = Vector((1, 1, 1))


def armature_delta_to_basis(arm: bpy.types.Object, bone_key: str, degrees: tuple[float, float, float]) -> Quaternion:
    bone = arm.data.bones[BONES[bone_key]]
    rest_rotation = bone.matrix_local.to_quaternion()
    armature_delta = Euler(radians3(degrees), "XYZ").to_quaternion()
    return rest_rotation.inverted() @ armature_delta @ rest_rotation


def armature_translation_to_basis(arm: bpy.types.Object, bone_key: str, value: tuple[float, float, float]) -> Vector:
    bone = arm.data.bones[BONES[bone_key]]
    rest_rotation = bone.matrix_local.to_quaternion()
    return rest_rotation.inverted() @ Vector(value)


def apply_pose(arm: bpy.types.Object, pose: dict) -> None:
    clear_pose(arm)
    for key, channels in pose.items():
        actual_key = "hips" if key == "hips_rot" else key
        if actual_key not in BONES:
            continue
        pose_bone = arm.pose.bones[BONES[actual_key]]
        rotation = Quaternion((1, 0, 0, 0))
        if "global" in channels:
            rotation = armature_delta_to_basis(arm, actual_key, channels["global"])
        if "local" in channels:
            rotation = rotation @ Euler(radians3(channels["local"]), "XYZ").to_quaternion()
        pose_bone.rotation_quaternion = rotation.normalized()
        if "location" in channels:
            pose_bone.location = armature_translation_to_basis(arm, actual_key, channels["location"])


def keyframe_humanoid(arm: bpy.types.Object, frame: int) -> None:
    for bone_name in BONES.values():
        pose_bone = arm.pose.bones[bone_name]
        pose_bone.keyframe_insert("rotation_quaternion", frame=frame, group=bone_name)
    hips = arm.pose.bones[BONES["hips"]]
    hips.keyframe_insert("location", frame=frame, group=BONES["hips"])


def action_fcurves(action: bpy.types.Action):
    """Yield F-curves from both Blender 4 legacy and Blender 5 layered Actions."""
    legacy_fcurves = getattr(action, "fcurves", None)
    if legacy_fcurves is not None:
        yield from legacy_fcurves
        return
    for layer in action.layers:
        for strip in layer.strips:
            for channelbag in strip.channelbags:
                yield from channelbag.fcurves


def configure_curves(action: bpy.types.Action, loop: bool) -> None:
    for fcurve in action_fcurves(action):
        for point in fcurve.keyframe_points:
            point.interpolation = "BEZIER"
            point.handle_left_type = "AUTO_CLAMPED"
            point.handle_right_type = "AUTO_CLAMPED"
        if loop:
            modifier = fcurve.modifiers.new("CYCLES")
            modifier.mode_before = "REPEAT"
            modifier.mode_after = "REPEAT"


def create_action(arm: bpy.types.Object, motion_id: str, spec: dict) -> bpy.types.Action:
    old = bpy.data.actions.get(f"yuukei::{motion_id}")
    if old:
        bpy.data.actions.remove(old)
    action = bpy.data.actions.new(f"yuukei::{motion_id}")
    action.use_fake_user = True
    action["yuukei_motion_id"] = motion_id
    action["yuukei_loop"] = bool(spec["loop"])
    action["yuukei_fps"] = FPS
    arm.animation_data_create()
    arm.animation_data.action = action
    for frame, pose in spec["frames"]:
        apply_pose(arm, pose)
        keyframe_humanoid(arm, frame)
    action.frame_start = spec["frames"][0][0]
    action.frame_end = spec["frames"][-1][0]
    configure_curves(action, bool(spec["loop"]))
    return action


def configure_preview_camera() -> None:
    camera = bpy.data.objects.get("Camera")
    if camera is None:
        camera_data = bpy.data.cameras.new("Camera")
        camera = bpy.data.objects.new("Camera", camera_data)
        bpy.context.scene.collection.objects.link(camera)
    camera.location = (2.4, -5.2, 2.1)
    camera.rotation_euler = (Vector((0, 0, 0.78)) - camera.location).to_track_quat("-Z", "Y").to_euler()
    camera.data.type = "ORTHO"
    camera.data.ortho_scale = 1.85
    scene = bpy.context.scene
    scene.camera = camera
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.resolution_x = 512
    scene.render.resolution_y = 640
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.film_transparent = True


def representative_frame(spec: dict) -> int:
    frames = [frame for frame, _ in spec["frames"]]
    if spec["loop"]:
        return frames[len(frames) // 2]
    return frames[max(0, len(frames) // 2)]


def render_previews(arm: bpy.types.Object, actions: dict[str, bpy.types.Action]) -> None:
    PREVIEW_DIR.mkdir(parents=True, exist_ok=True)
    configure_preview_camera()
    scene = bpy.context.scene
    for motion_id, action in actions.items():
        arm.animation_data.action = action
        frame = representative_frame(MOTIONS[motion_id])
        scene.frame_set(frame)
        scene.render.filepath = str(PREVIEW_DIR / f"{motion_id}.png")
        bpy.ops.render.render(write_still=True)


def export_vrma(arm: bpy.types.Object, actions: dict[str, bpy.types.Action]) -> dict[str, str]:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    scene = bpy.context.scene
    exported = {}
    for motion_id, action in actions.items():
        arm.animation_data.action = action
        scene.frame_start = int(action.frame_start)
        scene.frame_end = int(action.frame_end)
        scene.frame_set(scene.frame_start)
        path = OUTPUT_DIR / f"{motion_id}.vrma"
        result = bpy.ops.export_scene.vrma(
            filepath=str(path),
            armature_object_name=arm.name,
            check_existing=False,
        )
        if "FINISHED" not in result:
            raise RuntimeError(f"VRMA export failed for {motion_id}: {result}")
        exported[motion_id] = str(path)
    return exported


def authored_angle_report() -> tuple[list[dict], list[dict]]:
    measurements = []
    violations = []
    for motion_id, spec in MOTIONS.items():
        for frame, pose in spec["frames"]:
            for bone_key, channels in pose.items():
                actual_key = "hips" if bone_key == "hips_rot" else bone_key
                if actual_key not in QA_LIMITS_DEG:
                    continue
                for coordinate_space in ("global", "local"):
                    degrees = channels.get(coordinate_space)
                    if not degrees:
                        continue
                    limits = QA_LIMITS_DEG[actual_key]
                    item = {
                        "motion": motion_id,
                        "frame": frame,
                        "bone": actual_key,
                        "coordinateSpace": coordinate_space,
                        "degrees": list(degrees),
                        "limits": list(limits),
                    }
                    measurements.append(item)
                    if any(abs(value) > limit for value, limit in zip(degrees, limits)):
                        violations.append(item)
    return measurements, violations


def loop_report(actions: dict[str, bpy.types.Action]) -> list[dict]:
    report = []
    for motion_id, spec in MOTIONS.items():
        if not spec["loop"]:
            continue
        action = actions[motion_id]
        start = int(action.frame_start)
        end = int(action.frame_end)
        maximum_difference = 0.0
        for fcurve in action_fcurves(action):
            start_value = fcurve.evaluate(start)
            end_value = fcurve.evaluate(end)
            maximum_difference = max(maximum_difference, abs(start_value - end_value))
        report.append(
            {
                "motion": motion_id,
                "start": start,
                "end": end,
                "max_endpoint_difference": maximum_difference,
                "pass": maximum_difference < 1e-5,
            }
        )
    return report


def validate_vrma_imports(arm: bpy.types.Object, exported: dict[str, str]) -> list[dict]:
    """Round-trip every exported file through the official Blender VRMA importer."""
    original_action = arm.animation_data.action
    report = []
    for motion_id, path in exported.items():
        actions_before = {action.name for action in bpy.data.actions}
        objects_before = {obj.name for obj in bpy.data.objects}
        try:
            result = bpy.ops.import_scene.vrma(
                filepath=path,
                armature_object_name=arm.name,
            )
            imported_action = arm.animation_data.action
            curve_count = (
                sum(1 for _ in action_fcurves(imported_action))
                if imported_action
                else 0
            )
            report.append(
                {
                    "motion": motion_id,
                    "operatorResult": sorted(result),
                    "curveCount": curve_count,
                    "frameRange": (
                        [
                            float(imported_action.frame_range[0]),
                            float(imported_action.frame_range[1]),
                        ]
                        if imported_action
                        else None
                    ),
                    "newObjectCount": len(
                        {obj.name for obj in bpy.data.objects} - objects_before
                    ),
                    "pass": "FINISHED" in result and curve_count > 0,
                }
            )
        finally:
            arm.animation_data.action = original_action
            for name in {action.name for action in bpy.data.actions} - actions_before:
                action = bpy.data.actions.get(name)
                if action:
                    bpy.data.actions.remove(action)
            for name in {obj.name for obj in bpy.data.objects} - objects_before:
                obj = bpy.data.objects.get(name)
                if obj:
                    bpy.data.objects.remove(obj, do_unlink=True)
    return report


def main() -> None:
    arm = armature()
    scene = bpy.context.scene
    scene.render.fps = FPS
    scene.render.fps_base = 1.0
    bpy.context.view_layer.objects.active = arm
    arm.select_set(True)

    actions = {
        motion_id: create_action(arm, motion_id, spec)
        for motion_id, spec in MOTIONS.items()
    }

    render_previews(arm, actions)
    exported = export_vrma(arm, actions)
    import_checks = validate_vrma_imports(arm, exported)
    measurements, violations = authored_angle_report()
    loops = loop_report(actions)
    report = {
        "armature": arm.name,
        "blenderVersion": bpy.app.version_string,
        "fps": FPS,
        "motionCount": len(actions),
        "motions": {
            motion_id: {
                "file": exported[motion_id],
                "loop": bool(MOTIONS[motion_id]["loop"]),
                "frameStart": int(action.frame_start),
                "frameEnd": int(action.frame_end),
                "durationSeconds": (action.frame_end - action.frame_start) / FPS,
            }
            for motion_id, action in actions.items()
        },
        "authoredAngleChecks": {
            "measurementCount": len(measurements),
            "violations": violations,
            "pass": not violations,
        },
        "loopEndpointChecks": loops,
        "vrmaImportChecks": {
            "pass": all(check["pass"] for check in import_checks),
            "results": import_checks,
        },
        "biomechanicsBasis": {
            "principles": [
                "Keep quiet-standing center of mass over the base of support and retain small postural sway.",
                "Coordinate arm swing with the contralateral leg during walking.",
                "Use hip, knee, and ankle flexion together for a soft landing; avoid knee valgus.",
                "Keep authored joint rotations below conservative active-ROM QA limits.",
                "Keep locomotion in-place because DesktopStageManager owns screen translation.",
            ],
            "sources": [
                "https://pmc.ncbi.nlm.nih.gov/articles/PMC4618057/",
                "https://pmc.ncbi.nlm.nih.gov/articles/PMC2817299/",
                "https://pmc.ncbi.nlm.nih.gov/articles/PMC2815098/",
                "https://pubmed.ncbi.nlm.nih.gov/21070485/",
                "https://vrm.dev/en/vrm/vrm_features/",
                "https://github.com/vrm-c/vrm-specification/tree/master/specification/VRMC_vrm_animation-1.0",
            ],
        },
    }
    QA_REPORT.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")

    arm.animation_data.action = actions["idle_breathe"]
    scene.frame_start = int(actions["idle_breathe"].frame_start)
    scene.frame_end = int(actions["idle_breathe"].frame_end)
    scene.frame_set(1)
    bpy.ops.wm.save_as_mainfile(filepath=str(AUTHORING_BLEND))
    print(json.dumps(report, ensure_ascii=False, indent=2))


main()
