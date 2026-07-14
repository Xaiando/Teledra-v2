import os
import re

def validate_windows_path(path_str, workspace_root):
    # Reject relative traversing
    if ".." in path_str: return False
    # Reject UNC paths
    if path_str.startswith("\\\\\\\\") or path_str.startswith("//"): return False
    # Reject device paths
    if path_str.startswith(r"\\?\\") or path_str.startswith(r"\\.\\"): return False
    if "GLOBALROOT" in path_str: return False
    # Reject alternate data streams (any colon after index 2)
    if ":" in path_str[2:]: return False
    
    # Reject trailing spaces/dots
    if path_str.endswith(" ") or path_str.endswith("."): return False
    
    # Check for reserved names (CON, PRN, AUX, etc.)
    reserved = {"CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"}
    basename = os.path.basename(path_str).split('.')[0].upper()
    if basename in reserved: return False
    
    # Runtime containment check
    try:
        resolved = os.path.realpath(path_str)
        base = os.path.realpath(workspace_root)
        if not resolved.startswith(base): return False
    except:
        return False
        
    return True
