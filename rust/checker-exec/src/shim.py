"""checker-exec shim: embedded port of checker_runner/runners/ctf.py (ADR-017).

Deliberate differences from the original (recorded in rust/DECISIONS.md ADR-017):
- Service config comes from argv (the worker owns the Service row) instead of a
  DB read; the "Service N not found in DB" path no longer exists.
- Packages load from disk only: no DBFilesystem fallback, no FileLock, so the
  one-shot "Package X loaded" line is gone.
- No database or redis wiring: init_database() and
  NamedRedisConnection.set_clientname() are not needed by checker execution.
- set_process_needs_restart() was a module-global flag checked by the celery
  parent process; in the original's subprocess path it was a no-op.

Output contract (unchanged): console output, then SEPARATOR + "status|message",
exit 0; uncaught errors (e.g. bad argv) raise and exit 1.
"""
import importlib.util
import os
import sys
import traceback

# Set environment for pwntools (as the original wrapper did)
os.environ["TERM"] = "xterm"
os.environ["PWNLIB_NOTERM"] = "1"

SEPARATOR = "\n\n" + "-" * 72


def load_checker_class(package, script, config):
    fname, clsname = script.split(":")
    if package:
        modulename = "{}.{}".format(package, fname.replace(".py", "").replace("/", "."))
        spec = importlib.util.spec_from_file_location(
            modulename, config.CHECKER_PACKAGES_PATH / package / fname)
        if spec is None:
            raise Exception("Spec/Loader is not present")
        module = importlib.util.module_from_spec(spec)
        if spec.loader is None:
            raise Exception("Loader is not present")
        spec.loader.exec_module(module)
        print("PackageLoader imported {}".format(modulename))
    else:
        module = importlib.import_module(fname)
    return getattr(module, clsname)


def execute(package, script, service_id, team_id, tick, name, flag_ids_csv):
    from gamelib import Team, ServiceConfig, gamelogger
    from gamelib.exceptions import handle_checker_exceptions
    from saarctf_commons.config import config, load_default_config

    load_default_config()
    sys.path.append(str(config.CHECKER_PACKAGES_PATH))

    service_config = ServiceConfig(
        name=name,
        service_id=service_id,
        flag_ids=flag_ids_csv.split(",") if flag_ids_csv else [],
        interface_file=script.split(':', 1)[0] if script else '',
        interface_class=script.split(':', 1)[1] if script and ':' in script else '',
    )

    def run_checker():
        team = Team(team_id, '#' + str(team_id), config.NETWORK.team_id_to_vulnbox_ip(team_id))
        gamelogger.GameLogger.reset()
        checker = load_checker_class(package, script, config)(service_config)
        checker.initialize_team(team)
        try:
            gamelogger.GameLogger.log("----- check_integrity -----")
            checker.check_integrity(team, tick)
            gamelogger.GameLogger.log(f"----- store_flags({tick}) -----")
            checker.store_flags(team, tick)
            if tick > 1:
                gamelogger.GameLogger.log(f"----- retrieve_flags({tick - 1}) -----")
                checker.retrieve_flags(team, tick - 1)
            elif tick <= -1:
                # Test run - retrieve the flag we just have set
                gamelogger.GameLogger.log(f"----- retrieve_flags({tick}) -----")
                checker.retrieve_flags(team, tick)
        finally:
            try:
                checker.finalize_team(team)
            except:
                traceback.print_exc()
        return ("SUCCESS", None)

    try:
        result = handle_checker_exceptions(run_checker)
        if isinstance(result, tuple):
            return result[0], result[1]
        return result.status, result.message
    except:
        traceback.print_exc()
        return "CRASHED", None


def main():
    print("(subprocess invoked)")
    if len(sys.argv) <= 7:
        raise Exception("Not enough arguments!")
    package, script = sys.argv[1], sys.argv[2]
    service_id, team_id, tick = int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
    name = sys.argv[6]
    flag_ids_csv = sys.argv[7]
    status, message = execute(package, script, service_id, team_id, tick, name, flag_ids_csv)
    print(SEPARATOR)
    print(status + "|" + (message or ""))
    # The embedded interpreter is not finalized, so flush like a real process exit would.
    sys.stdout.flush()
