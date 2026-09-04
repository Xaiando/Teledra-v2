# Tower

Owns the local inbox and archive for isolated or remote resident reports.
Resident and report identifiers are path-safe. Reports remain provenance-only
until another subsystem explicitly imports them; residents never share mutable
court internals or silently write into another subsystem.
