import { useEffect, useRef, useState } from "react";
import { commands, type LoginItemStatus } from "../bindings";
import { Button, ErrorText, Row, Switch } from "./ui";

export function LoginItem() {
  const [status, setStatus] = useState<LoginItemStatus>("unavailable");
  const [pending, setPending] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);
  const changing = useRef(false);

  useEffect(() => {
    let mounted = true;
    const refresh = async () => {
      if (changing.current) return;
      const current = ++request.current;
      setPending(true);
      try {
        const result = await commands.loginItemStatus();
        if (!mounted || current !== request.current) return;
        setStatus(result.status === "ok" ? result.data : "unavailable");
        setError(result.status === "error" ? result.error : null);
      } catch (error) {
        if (!mounted || current !== request.current) return;
        setStatus("unavailable");
        setError(String(error));
      } finally {
        if (mounted && current === request.current) setPending(false);
      }
    };
    void refresh();
    window.addEventListener("focus", refresh);
    return () => {
      mounted = false;
      ++request.current;
      window.removeEventListener("focus", refresh);
    };
  }, []);

  const change = async (enabled: boolean) => {
    if (changing.current) return;
    changing.current = true;
    const current = ++request.current;
    setPending(true);
    setError(null);
    try {
      const result = await commands.setLoginItem(enabled);
      if (current !== request.current) return;
      if (result.status === "ok") {
        setStatus(result.data);
      } else {
        setError(result.error);
        const observed = await commands.loginItemStatus();
        if (current === request.current) setStatus(observed.status === "ok" ? observed.data : "unavailable");
      }
    } catch (error) {
      if (current === request.current) {
        setStatus("unavailable");
        setError(String(error));
      }
    } finally {
      changing.current = false;
      if (current === request.current) setPending(false);
    }
  };

  const openSettings = async () => {
    try {
      const result = await commands.openLoginItemSettings();
      if (result.status === "error") setError(result.error);
    } catch (error) {
      setError(String(error));
    }
  };

  let detail = "Runs in the background without opening a window or recording";
  if (pending) {
    detail = changing.current ? "Updating…" : "Checking…";
  } else if (status === "approval_required") {
    detail = "Allow Longhand in System Settings to start at login";
  } else if (status === "unavailable") {
    detail = "Start at login is unavailable";
  }

  return (
    <>
      <Row label="Start at login" detail={detail}>
        <Switch
          label="Start at login"
          checked={status === "enabled"}
          disabled={pending || status === "unavailable" || status === "approval_required"}
          onChange={next => void change(next)}
        />
      </Row>
      {status === "approval_required" && (
        <Row label="Login approval">
          <Button disabled={pending} onClick={() => void change(false)}>Cancel Request</Button>
          <Button onClick={() => void openSettings()}>Open System Settings</Button>
        </Row>
      )}
      {error && <ErrorText>{error}</ErrorText>}
    </>
  );
}
