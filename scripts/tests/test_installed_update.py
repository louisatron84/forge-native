import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('installed_update', Path(__file__).parents[1] / 'test_installed_update.py')
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


class InstalledUpdateGuardTests(unittest.TestCase):
    def test_installation_fixture_refuses_a_personal_or_self_hosted_machine(self):
        for environment in ({}, {'GITHUB_ACTIONS': 'true', 'RUNNER_ENVIRONMENT': 'self-hosted'}):
            with self.subTest(environment=environment), patch.dict(smoke.os.environ, environment, clear=True):
                with self.assertRaisesRegex(RuntimeError, 'disposable GitHub-hosted Windows'):
                    smoke.main()
