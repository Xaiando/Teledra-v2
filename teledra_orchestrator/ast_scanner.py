import ast
import sys

FORBIDDEN_MODULES = {
    "subprocess", "os", "socket", "requests", "urllib", "ctypes", "pip", "ensurepip", "importlib", "sys"
}

FORBIDDEN_CALLS = {
    "os.system", "os.popen", "subprocess.Popen", "subprocess.call", "subprocess.check_call", "subprocess.run", "eval", "exec", "open"
}

class SandboxSecurityScanner(ast.NodeVisitor):
    def __init__(self):
        self.violations = []

    def visit_Import(self, node):
        for alias in node.names:
            base_module = alias.name.split('.')[0]
            if base_module in FORBIDDEN_MODULES:
                self.violations.append(f"Forbidden module import: {alias.name}")
        self.generic_visit(node)

    def visit_ImportFrom(self, node):
        if node.module:
            base_module = node.module.split('.')[0]
            if base_module in FORBIDDEN_MODULES:
                self.violations.append(f"Forbidden module import from: {node.module}")
        self.generic_visit(node)

    def visit_Call(self, node):
        if isinstance(node.func, ast.Name):
            if node.func.id in FORBIDDEN_CALLS:
                self.violations.append(f"Forbidden function call: {node.func.id}")
        elif isinstance(node.func, ast.Attribute):
            if isinstance(node.func.value, ast.Name):
                full_call = f"{node.func.value.id}.{node.func.attr}"
                if full_call in FORBIDDEN_CALLS:
                    self.violations.append(f"Forbidden method call: {full_call}")
        self.generic_visit(node)

def scan_file(filepath):
    with open(filepath, "r", encoding="utf-8") as f:
        source = f.read()
    
    try:
        tree = ast.parse(source)
    except SyntaxError as e:
        return [f"SyntaxError: {e}"]
        
    scanner = SandboxSecurityScanner()
    scanner.visit(tree)
    return scanner.violations

if __name__ == '__main__':
    if len(sys.argv) < 2:
        print("Usage: python ast_scanner.py <file.py>")
        sys.exit(1)
        
    violations = scan_file(sys.argv[1])
    if violations:
        print("SANDBOX_POLICY_DENIAL")
        for v in violations:
            print(f"- {v}")
        sys.exit(1)
    else:
        print("PASS")
        sys.exit(0)
