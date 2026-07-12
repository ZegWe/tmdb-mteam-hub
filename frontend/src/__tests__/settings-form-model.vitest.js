import { describe, expect, it } from "vitest";
import {
  settingsFormFromSnapshot,
  settingsUpdatePayload,
  qbServerPatch,
  qbTestPayload,
  secretPatchFields,
} from "../features/settings/form-model.js";

describe("settings form security model", () => {
  it("uses keep, replace, and explicit clear semantics for redacted secrets", () => {
    expect(
      secretPatchFields({
        value: "",
        clear: false,
        valueField: "tmdb_api_key",
        clearField: "clear_tmdb_api_key",
      }),
    ).toEqual({});
    expect(
      secretPatchFields({
        value: "new-secret",
        clear: false,
        valueField: "tmdb_api_key",
        clearField: "clear_tmdb_api_key",
      }),
    ).toEqual({ tmdb_api_key: "new-secret" });
    expect(
      secretPatchFields({
        value: "must-not-be-sent",
        clear: true,
        valueField: "tmdb_api_key",
        clearField: "clear_tmdb_api_key",
      }),
    ).toEqual({ clear_tmdb_api_key: true });
  });

  it("keeps management token redacted and emits only explicit set or clear patches", () => {
    const model = settingsFormFromSnapshot({
      revision: 9,
      has_admin_token: true,
    });
    expect(model.form.admin_token).toBe("");
    expect(model.secretPresence.admin_token).toBe(true);

    expect(
      settingsUpdatePayload({
        form: model.form,
        expectedRevision: model.revision,
        clearSecrets: model.clearSecrets,
      }),
    ).not.toHaveProperty("admin_token");

    model.form.admin_token = "replacement-management-token-123456";
    expect(
      settingsUpdatePayload({
        form: model.form,
        expectedRevision: model.revision,
        clearSecrets: model.clearSecrets,
      }),
    ).toMatchObject({ admin_token: "replacement-management-token-123456" });

    model.clearSecrets.admin_token = true;
    expect(
      settingsUpdatePayload({
        form: model.form,
        expectedRevision: model.revision,
        clearSecrets: model.clearSecrets,
      }),
    ).toMatchObject({ clear_admin_token: true });
  });

  it("sends only a saved qB server ID to connection tests", () => {
    const payload = qbTestPayload({
      id: " nas ",
      base_url: "http://127.0.0.1:8080",
      username: "admin",
      password: "SECRET_MUST_NOT_BE_SENT",
      insecure_tls: true,
    });

    expect(payload).toEqual({ server_id: "nas" });
    expect(JSON.stringify(payload)).not.toContain("SECRET_MUST_NOT_BE_SENT");
  });

  it("keeps an existing qB password when no replacement is entered", () => {
    expect(
      qbServerPatch({
        id: "nas",
        name: "NAS",
        base_url: "http://127.0.0.1:8080",
        username: "admin",
        insecure_tls: false,
        has_password: true,
        password: "",
      }),
    ).toEqual({
      id: "nas",
      name: "NAS",
      base_url: "http://127.0.0.1:8080",
      username: "admin",
      insecure_tls: false,
    });
  });

  it("keeps qB password replacement and explicit clear distinct", () => {
    expect(qbServerPatch({ password: "new-secret" }).password).toBe("new-secret");
    expect(qbServerPatch({ password: "ignored", clear_password: true })).toMatchObject({
      clear_password: true,
    });
    expect(qbServerPatch({ password: "ignored", clear_password: true })).not.toHaveProperty(
      "password",
    );
  });
});
