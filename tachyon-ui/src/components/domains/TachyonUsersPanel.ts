import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
import { trapFocus } from "../../utils/a11y";
import { el } from "../../utils/dom-safe";
import { resilientInvoke as invoke } from "../../utils/network";
import { t } from "../../utils/i18n";

type UserSummary = {
  username: string;
  firstName: string;
  lastName: string;
  roles: string[];
  scopes: string[];
  groups: string[];
  disabledAt?: number | null;
  createdAt: number;
  lastLoginAt?: number | null;
};

type GroupSummary = {
  name: string;
  description: string;
  roles: string[];
  scopes: string[];
  memberCount: number;
  createdAt: number;
  updatedAt: number;
};

type AuditEntry = {
  timestamp: number;
  actor: string;
  targetUser?: string | null;
  targetGroup?: string | null;
  action: string;
  outcome: string;
  detail: string;
};

type UserUpdate = {
  addGroups?: string[];
  removeGroups?: string[];
  addRoles?: string[];
  removeRoles?: string[];
  addScopes?: string[];
  removeScopes?: string[];
  disabled?: boolean;
};

export class TachyonUsersPanel extends TachyonConfigDashboard {
  private users: UserSummary[] = [];
  private groups: GroupSummary[] = [];
  private auditEntries: AuditEntry[] = [];
  private auditTarget: string | null = null;
  private readonly onLanguageChanged = () => this.render();

  async connectedCallback(): Promise<void> {
    window.addEventListener("i18n:language-changed", this.onLanguageChanged);
    this.render();
    this.bindEvents();
    this.animateEntrance();
    await this.refresh();
  }

  disconnectedCallback(): void {
    window.removeEventListener("i18n:language-changed", this.onLanguageChanged);
  }

  private async refresh(): Promise<void> {
    const [users, groups] = await Promise.all([
      invoke<UserSummary[]>("iam_list_users").catch(() => []),
      invoke<GroupSummary[]>("iam_list_groups").catch(() => []),
    ]);
    this.users = users;
    this.groups = groups;
    this.render();
    this.bindEvents();
  }

  private render(): void {
    this.renderTemplate(`
      <section class="p-6 space-y-6 text-slate-300">
        <header data-stagger-panel class="flex items-end justify-between gap-4 border-l-4 border-cyan-500 pl-4">
          <div>
            <h2 class="text-2xl font-bold text-slate-100">${t("users.title")}</h2>
            <p class="text-sm font-mono text-slate-400">${t("users.subtitle")}</p>
          </div>
          <button id="btn-refresh-users" type="button" class="rounded-md border border-cyan-500/40 bg-cyan-500/10 px-3 py-2 text-xs font-medium text-cyan-200 hover:bg-cyan-500/20">${t("users.refresh")}</button>
        </header>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900 p-5">
          <h3 class="mb-3 text-sm font-semibold uppercase tracking-widest text-cyan-300">${t("users.list.title")}</h3>
          ${this.renderUserTable()}
        </article>

        <article data-stagger-panel class="rounded-lg border border-slate-800 bg-slate-900 p-5">
          <h3 class="mb-3 text-sm font-semibold uppercase tracking-widest text-cyan-300">${t("groups.title")}</h3>
          ${this.renderGroupSection()}
        </article>

        ${this.auditTarget ? this.renderAuditModal() : ""}

        <div id="feedback-zone" data-stagger-panel class="rounded-xl border border-slate-800 bg-slate-900 px-4 py-3 font-mono text-xs text-slate-400">${t("users.feedback.empty")}</div>
      </section>
    `);
    this.populateUserRows();
    this.populateGroupList();
    this.populateAuditModal();
  }

  private renderUserTable(): string {
    if (this.users.length === 0) {
      return `<p class="text-xs text-slate-500">${t("users.list.empty")}</p>`;
    }
    // Static structure — rows populated via DOM API.
    return `
      <table class="w-full text-xs">
        <thead class="text-slate-500 uppercase tracking-widest">
          <tr>
            <th class="text-left pb-2 pr-4">${t("users.column.username")}</th>
            <th class="text-left pb-2 pr-4">${t("users.column.status")}</th>
            <th class="text-left pb-2 pr-4">${t("users.column.groups")}</th>
            <th class="text-left pb-2 pr-4">${t("users.column.roles")}</th>
            <th class="text-left pb-2 pr-4">${t("users.column.last-login")}</th>
            <th class="text-right pb-2">${t("users.column.actions")}</th>
          </tr>
        </thead>
        <tbody id="users-rows"></tbody>
      </table>
    `;
  }

  private populateUserRows(): void {
    const tbody = this.root.getElementById("users-rows");
    if (!tbody) return;
    tbody.replaceChildren(
      ...this.users.map((user) => {
        const status = user.disabledAt
          ? el("span", { class: "text-amber-300" }, t("users.status.disabled"))
          : el("span", { class: "text-emerald-300" }, t("users.status.active"));
        const last = user.lastLoginAt
          ? new Date(user.lastLoginAt * 1000).toISOString().slice(0, 19).replace("T", " ")
          : "—";
        const groups = user.groups.length === 0 ? "—" : user.groups.join(", ");
        const roles = user.roles.length === 0 ? "—" : user.roles.join(", ");
        const toggleLabel = user.disabledAt ? t("users.action.enable") : t("users.action.disable");

        const actionBtn = (action: string, label: string, cls: string) =>
          el("button", {
            "data-action": action,
            "data-target": user.username,
            class: cls,
          }, label);

        return el("tr", { "data-user": user.username, class: "border-t border-slate-800" },
          el("td", { class: "py-2 pr-4 font-mono text-cyan-300" }, user.username),
          el("td", { class: "py-2 pr-4" }, status),
          el("td", { class: "py-2 pr-4 text-slate-300" }, groups),
          el("td", { class: "py-2 pr-4 text-slate-300" }, roles),
          el("td", { class: "py-2 pr-4 font-mono text-slate-400" }, last),
          el("td", { class: "py-2 text-right" },
            el("div", { class: "flex flex-wrap justify-end gap-1" },
              actionBtn("toggle-disabled", toggleLabel, "rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-700"),
              actionBtn("edit-groups", t("users.action.edit-groups"), "rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-700"),
              actionBtn("edit-roles", t("users.action.edit-roles"), "rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-700"),
              actionBtn("view-audit", t("users.action.view-audit"), "rounded border border-cyan-500/40 bg-cyan-500/10 px-2 py-1 text-[11px] text-cyan-200 hover:bg-cyan-500/20"),
              actionBtn("delete", t("users.action.delete"), "rounded border border-red-500/40 bg-red-500/10 px-2 py-1 text-[11px] text-red-200 hover:bg-red-500/20"),
            ),
          ),
        );
      }),
    );
  }

  private renderGroupSection(): string {
    // Static structure — list items populated via DOM API.
    return `
      <div class="grid gap-4 lg:grid-cols-[1.4fr_1fr]">
        <div id="groups-list-host"></div>
        <form id="group-form" class="space-y-3 rounded border border-slate-800 bg-slate-950/40 p-4">
          <h4 class="text-xs uppercase tracking-widest text-cyan-300">${t("groups.create.title")}</h4>
          <input id="group-name" type="text" placeholder="${t("groups.create.name")}" class="w-full rounded border border-slate-700 bg-slate-950 p-2 text-sm text-slate-200 outline-none focus:border-cyan-400" />
          <input id="group-description" type="text" placeholder="${t("groups.create.description")}" class="w-full rounded border border-slate-700 bg-slate-950 p-2 text-sm text-slate-200 outline-none focus:border-cyan-400" />
          <input id="group-roles" type="text" placeholder="${t("groups.create.roles")}" class="w-full rounded border border-slate-700 bg-slate-950 p-2 text-sm text-slate-200 outline-none focus:border-cyan-400" />
          <input id="group-scopes" type="text" placeholder="${t("groups.create.scopes")}" class="w-full rounded border border-slate-700 bg-slate-950 p-2 text-sm text-slate-200 outline-none focus:border-cyan-400" />
          <button type="submit" class="w-full rounded border border-cyan-500/50 bg-cyan-500/15 px-3 py-2 text-xs font-medium text-cyan-200 hover:bg-cyan-500/25">${t("groups.create.submit")}</button>
        </form>
      </div>
    `;
  }

  private populateGroupList(): void {
    const host = this.root.getElementById("groups-list-host");
    if (!host) return;

    if (this.groups.length === 0) {
      host.replaceChildren(el("p", { class: "text-xs text-slate-500" }, t("groups.empty")));
      return;
    }

    const tagSpan = (text: string) =>
      el("span", { class: "rounded bg-slate-800 px-2 py-0.5 text-slate-200" }, text);
    const italicSpan = (text: string) =>
      el("span", { class: "italic text-slate-600" }, text);

    const items = this.groups.map((group) => {
      const description = group.description
        ? el("p", { class: "mt-1 text-xs text-slate-400" }, group.description)
        : el("p", { class: "mt-1 text-xs text-slate-400" }, italicSpan(t("groups.no-description")));

      const rolesRow = el("div", { class: "mt-2 flex flex-wrap gap-2 text-[11px]" },
        el("span", { class: "text-slate-500" }, `${t("groups.roles")}:`),
        ...(group.roles.length > 0
          ? group.roles.map(tagSpan)
          : [italicSpan(t("groups.no-roles"))]),
      );

      const scopesRow = el("div", { class: "mt-1 flex flex-wrap gap-2 text-[11px]" },
        el("span", { class: "text-slate-500" }, `${t("groups.scopes")}:`),
        ...(group.scopes.length > 0
          ? group.scopes.map(tagSpan)
          : [italicSpan(t("groups.no-scopes"))]),
      );

      const header = el("div", { class: "flex items-center justify-between gap-3" },
        el("div", {},
          el("span", { class: "font-mono text-sm text-cyan-300" }, group.name),
          el("span", { class: "ml-2 text-[10px] uppercase tracking-widest text-slate-500" }, `${group.memberCount} ${t("groups.members")}`),
        ),
        el("div", { class: "flex gap-1" },
          el("button", {
            "data-group-action": "edit",
            "data-target": group.name,
            class: "rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-700",
          }, t("groups.action.edit")),
          el("button", {
            "data-group-action": "delete",
            "data-target": group.name,
            class: "rounded border border-red-500/40 bg-red-500/10 px-2 py-1 text-[11px] text-red-200 hover:bg-red-500/20",
          }, t("groups.action.delete")),
        ),
      );

      return el("li", { class: "rounded border border-slate-800 bg-slate-950/40 p-3" },
        header, description, rolesRow, scopesRow,
      );
    });

    host.replaceChildren(el("ul", { class: "space-y-2" }, ...items));
  }

  private renderAuditModal(): string {
    // Static structure — title target and rows populated via DOM API.
    return `
      <div id="audit-modal-overlay" role="dialog" aria-modal="true" aria-labelledby="audit-modal-title" class="fixed inset-0 z-[80] flex items-center justify-center bg-slate-950/80 backdrop-blur-sm">
        <div class="w-[min(48rem,calc(100vw-2rem))] rounded-lg border border-slate-700 bg-slate-900 p-5">
          <div class="mb-3 flex items-center justify-between">
            <h3 id="audit-modal-title" class="text-sm font-semibold text-cyan-300">${t("users.audit.title")} <span id="audit-modal-target" class="ml-2 font-mono text-slate-300"></span></h3>
            <button id="btn-close-audit" type="button" class="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-xs text-slate-300 hover:bg-slate-700">${t("users.audit.close")}</button>
          </div>
          <div class="max-h-[60vh] overflow-y-auto">
            <table class="w-full text-xs">
              <thead class="text-slate-500 uppercase tracking-widest">
                <tr>
                  <th class="text-left pb-2 pr-3">${t("users.audit.column.timestamp")}</th>
                  <th class="text-left pb-2 pr-3">${t("users.audit.column.action")}</th>
                  <th class="text-left pb-2 pr-3">${t("users.audit.column.outcome")}</th>
                  <th class="text-left pb-2">${t("users.audit.column.detail")}</th>
                </tr>
              </thead>
              <tbody id="audit-rows"></tbody>
            </table>
          </div>
        </div>
      </div>
    `;
  }

  private populateAuditModal(): void {
    if (!this.auditTarget) return;
    const target = this.root.getElementById("audit-modal-target");
    if (target) target.textContent = this.auditTarget;

    const tbody = this.root.getElementById("audit-rows");
    if (!tbody) return;

    if (this.auditEntries.length === 0) {
      tbody.replaceChildren(
        el("tr", {},
          el("td", { class: "py-4 text-center text-xs text-slate-500", colspan: "4" }, t("users.audit.empty")),
        ),
      );
      return;
    }

    tbody.replaceChildren(
      ...this.auditEntries.map((entry) => {
        const ts = new Date(entry.timestamp * 1000).toISOString().slice(0, 19).replace("T", " ");
        const outcomeColor = entry.outcome === "ok" ? "text-emerald-300" : "text-amber-300";
        return el("tr", { class: "border-t border-slate-800" },
          el("td", { class: "py-1 pr-3 font-mono text-slate-400" }, ts),
          el("td", { class: "py-1 pr-3 text-cyan-300" }, entry.action),
          el("td", { class: `py-1 pr-3 ${outcomeColor}` }, entry.outcome),
          el("td", { class: "py-1 text-slate-300" }, entry.detail || "—"),
        );
      }),
    );
  }

  private bindEvents(): void {
    this.root.getElementById("btn-refresh-users")?.addEventListener("click", () => {
      void this.refresh();
    });

    this.root.querySelectorAll<HTMLButtonElement>("button[data-action]").forEach((button) => {
      button.addEventListener("click", () => {
        const action = button.dataset.action;
        const target = button.dataset.target;
        if (!action || !target) return;
        switch (action) {
          case "toggle-disabled":
            void this.toggleDisabled(target);
            break;
          case "edit-groups":
            void this.editMembership(target, "groups");
            break;
          case "edit-roles":
            void this.editMembership(target, "roles");
            break;
          case "view-audit":
            void this.viewAudit(target);
            break;
          case "delete":
            void this.deleteUser(target);
            break;
        }
      });
    });

    this.root.querySelectorAll<HTMLButtonElement>("button[data-group-action]").forEach((button) => {
      button.addEventListener("click", () => {
        const action = button.dataset.groupAction;
        const target = button.dataset.target;
        if (!action || !target) return;
        if (action === "delete") void this.deleteGroup(target);
        if (action === "edit") void this.editGroup(target);
      });
    });

    this.root.getElementById("btn-close-audit")?.addEventListener("click", () => {
      this.auditTarget = null;
      this.auditEntries = [];
      this.render();
      this.bindEvents();
    });

    this.root.getElementById("group-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.submitGroupForm();
    });
  }

  private async toggleDisabled(username: string): Promise<void> {
    const user = this.users.find((u) => u.username === username);
    if (!user) return;
    const update: UserUpdate = { disabled: !user.disabledAt };
    try {
      await invoke<UserSummary>("iam_update_user", { username, update });
      this.showFeedback("success", t("users.feedback.updated"));
      await this.refresh();
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private async editMembership(username: string, kind: "groups" | "roles"): Promise<void> {
    const user = this.users.find((u) => u.username === username);
    if (!user) return;
    const current = (kind === "groups" ? user.groups : user.roles).join(", ");
    const promptKey = kind === "groups" ? "users.prompt.groups" : "users.prompt.roles";
    const next = window.prompt(t(promptKey), current);
    if (next === null) return;
    const desired = next
      .split(",")
      .map((value) => value.trim())
      .filter((value) => value.length > 0);
    const currentSet = new Set(kind === "groups" ? user.groups : user.roles);
    const desiredSet = new Set(desired);
    const add = desired.filter((value) => !currentSet.has(value));
    const remove = (kind === "groups" ? user.groups : user.roles).filter((value) => !desiredSet.has(value));
    const update: UserUpdate =
      kind === "groups"
        ? { addGroups: add, removeGroups: remove }
        : { addRoles: add, removeRoles: remove };
    try {
      await invoke<UserSummary>("iam_update_user", { username, update });
      this.showFeedback("success", t("users.feedback.updated"));
      await this.refresh();
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private async deleteUser(username: string): Promise<void> {
    if (!window.confirm(t("users.confirm.delete").replace("{name}", username))) return;
    try {
      await invoke("iam_delete_user", { username });
      this.showFeedback("success", t("users.feedback.deleted"));
      await this.refresh();
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private async viewAudit(username: string): Promise<void> {
    try {
      this.auditEntries = await invoke<AuditEntry[]>("fetch_user_audit_log", {
        user: username,
        lines: 200,
      });
      this.auditTarget = username;
    } catch {
      this.auditEntries = [];
      this.auditTarget = username;
    }
    this.render();
    this.bindEvents();
    // Trap focus inside the audit dialog; Escape closes it.
    const overlay = this.root.getElementById("audit-modal-overlay");
    if (overlay) {
      trapFocus(overlay, () => {
        this.auditTarget = null;
        this.auditEntries = [];
        this.render();
        this.bindEvents();
      });
    }
  }

  private async deleteGroup(name: string): Promise<void> {
    if (!window.confirm(t("groups.confirm.delete").replace("{name}", name))) return;
    try {
      await invoke("iam_delete_group", { name });
      this.showFeedback("success", t("groups.feedback.deleted"));
      await this.refresh();
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private editGroup(name: string): void {
    const group = this.groups.find((g) => g.name === name);
    if (!group) return;
    this.setGroupForm(group);
  }

  private async submitGroupForm(): Promise<void> {
    const input = {
      name: (this.root.getElementById("group-name") as HTMLInputElement | null)?.value.trim() ?? "",
      description:
        (this.root.getElementById("group-description") as HTMLInputElement | null)?.value.trim() ??
        "",
      roles: this.parseList(
        (this.root.getElementById("group-roles") as HTMLInputElement | null)?.value ?? "",
      ),
      scopes: this.parseList(
        (this.root.getElementById("group-scopes") as HTMLInputElement | null)?.value ?? "",
      ),
    };
    if (!input.name) {
      this.showFeedback("error", t("groups.error.name-required"));
      return;
    }
    try {
      await invoke<GroupSummary>("iam_upsert_group", { input });
      this.showFeedback("success", t("groups.feedback.upserted"));
      this.setGroupForm(null);
      await this.refresh();
    } catch (error) {
      this.showFeedback("error", error instanceof Error ? error.message : String(error));
    }
  }

  private setGroupForm(group: GroupSummary | null): void {
    const name = this.root.getElementById("group-name") as HTMLInputElement | null;
    const desc = this.root.getElementById("group-description") as HTMLInputElement | null;
    const roles = this.root.getElementById("group-roles") as HTMLInputElement | null;
    const scopes = this.root.getElementById("group-scopes") as HTMLInputElement | null;
    if (!name || !desc || !roles || !scopes) return;
    name.value = group?.name ?? "";
    desc.value = group?.description ?? "";
    roles.value = group?.roles.join(", ") ?? "";
    scopes.value = group?.scopes.join(", ") ?? "";
  }

  private parseList(value: string): string[] {
    return value
      .split(",")
      .map((item) => item.trim())
      .filter((item) => item.length > 0);
  }
}

customElements.define("tachyon-users-panel", TachyonUsersPanel);
