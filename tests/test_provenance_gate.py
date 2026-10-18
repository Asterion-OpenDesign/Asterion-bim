import hashlib, os, sys, tempfile, unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "tools"))
import provenance_gate as gate  # noqa: E402


def write(root, rel, text):
    p = os.path.join(root, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8") as fh:
        fh.write(text)
    return p


class GateTests(unittest.TestCase):
    def rules_hit(self, text, name="a.rs"):
        with tempfile.TemporaryDirectory() as d:
            write(d, name, text)
            return {r for _, _, r in gate.scan([d])}

    def test_clean_text_passes(self):
        self.assertEqual(self.rules_hit("// compute the area of a polygon\nfn area() {}\n"), set())

    def test_decompiler_name(self):
        self.assertIn("decompiler-routine-name", self.rules_hit("// see " + "FUN" + "_" + "180510620\n"))

    def test_native_address(self):
        self.assertIn("native-address", self.rules_hit("// at " + "0x" + "1804a7b10\n"))

    def test_hash_constant_is_not_an_address(self):
        self.assertNotIn("native-address", self.rules_hit("let p = " + "0x" + "100000001b3;\n"))

    def test_dll_name_mention(self):
        self.assertIn("dll-name-mention", self.rules_hit("// in " + "Some" + "Module" + ".dll\n"))

    def test_vendor_class_method(self):
        self.assertIn("vendor-class-method", self.rules_hit("// " + "VW" + "all::getLength\n"))

    def test_local_path(self):
        self.assertIn("local-machine-path", self.rules_hit("path = 'C:" + "\\Users\\someone\\x'\n"))

    def test_personal_email(self):
        self.assertIn("personal-email", self.rules_hit("// me@" + "gmail" + ".com\n"))

    def test_noreply_email_allowed(self):
        self.assertEqual(self.rules_hit("// 1+x@users.noreply.github.com\n"), set())

    def test_forbidden_extension_and_dir(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, "a/model.rvt", "x")
            write(d, "research/notes.txt", "x")
            got = {r for _, _, r in gate.scan([d])}
        self.assertEqual(got, {"forbidden-file-type", "forbidden-directory"})

    def test_allow_marker(self):
        self.assertEqual(self.rules_hit("// " + "FUN" + "_" + "180510620 provenance-gate: allow\n"), set())

    def test_private_denylist(self):
        tok = "SomeInternalName"
        h = {hashlib.sha256(tok.encode()).hexdigest()}
        with tempfile.TemporaryDirectory() as d:
            write(d, "a.rs", "// uses " + tok + " here\n")
            self.assertIn("private-denylist", {r for _, _, r in gate.scan([d], h)})
            self.assertEqual(gate.scan([d], None), [])


if __name__ == "__main__":
    unittest.main()
