import { TachyonConfigDashboard } from "../base/TachyonConfigDashboard";
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
    try {
      this.users = await invoke<UserSummary[]>("iam_list_users");
    } catch {
      this.users = [];
    }
    try {
      this.groups = await invoke<GroupSummary[]>("iam_list_groups");
    } catch {
      this.groups = [];
    }
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
  }

  private renderUserTable(): string {
    if (this.users.length === 0) {
      return `<p class="text-xs text-slate-500">${t("users.list.empty")}</p>`;
    }
    const rows = this.users
      .map((user) => {
        const status = user.disabledAt
          ? `<span class="text-amber-300">${t("users.status.disabled")}</span>`
          : `<span class="text-emerald-300">${t("users.status.active")}</span>`;
        const last = user.lastLoginAt
          ? new Date(user.lastLoginAt * 1000).toISOString().slice(0, 19).replace("T", " ")
          : "—";
        const groups = user.groups.length === 0 ? "—" : user.groups.map((g) => this.escape(g)).join(", ");
        const roles = user.roles.length === 0 ? "—" : user.roles.map((r) => this.escape(r)).join(", ");
        const toggleLabel = user.disabledAt ? t("users.action.enable") : t("users.action.disable");
        return `
          <tr data-user="${this.escape(user.username)}" class="border-t border-slate-800">
            <td class="py-2 pr-4 font-mono text-cyan-300">${this.escape(user.username)}</td>
            <td class="py-2 pr-4">${status}</td>
            <td class="py-2 pr-4 text-slate-300">${groups}</td>
            <td class="py-2 pr-4 text-slate-300">${roles}</td>
            <td class="py-2 pr-4 font-mono text-slate-400">${last}</td>
            <td class="py-2 text-right">
              <div class="flex flex-wrap justify-end gap-1">
                <button data-action="toggle-disabled" data-target="${this.escape(user.username)}" class="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-700">${toggleLabel}</button>
                <button data-action="edit-groups" data-target="${this.escape(user.username)}" class="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-700">${t("users.action.edit-groups")}</button>
                <button data-action="edit-roles" data-target="${this.escape(user.username)}" class="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-700">${t("users.action.edit-roles")}</button>
                <button data-action="view-audit" data-target="${this.escape(user.username)}" class="rounded border border-cyan-500/40 bg-cyan-500/10 px-2 py-1 text-[11px] text-cyan-200 hover:bg-cyan-500/20">${t("users.action.view-audit")}</button>
                <button data-action="delete" data-target="${this.escape(user.username)}" class="rounded border border-red-500/40 bg-red-500/10 px-2 py-1 text-[11px] text-red-200 hover:bg-red-500/20">${t("users.action.delete")}</button>
              </div>
            </td>
          </tr>
        `;
      })
      .join("");
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
        <tbody>${rows}</tbody>
      </table>
    `;
  }

  private renderGroupSection(): string {
    const list =
      this.groups.length === 0
        ? `<p class="text-xs text-slate-500">${t("groups.empty")}</p>`
        : `<ul class="space-y-2">${this.groups
            .map(
              (group) => `
              <li class="rounded border border-slate-800 bg-slate-950/40 p-3">
                <div class="flex items-center justify-between gap-3">
                  <div>
                    <span class="font-mono text-sm text-cyan-300">${this.escape(group.name)}</span>
                    <span class="ml-2 text-[10px] uppercase tracking-widest text-slate-500">${group.memberCount} ${t("groups.members")}</span>
                  </div>
                  <div class="flex gap-1">
                    <button data-group-action="edit" data-target="${this.escape(group.name)}" class="rounded border border-slate-700 bg-slate-800 px-2 py-1 text-[11px] text-slate-200 hover:bg-slate-700">${t("groups.action.edit")}</button>
                    <button data-group-action="delete" data-target="${this.escape(group.name)}" class="rounded border border-red-500/40 bg-red-500/10 px-2 py-1 text-[11px] text-red-200 hover:bg-red-500/20">${t("groups.action.delete")}</button>
                  </div>
                </div>
                <p class="mt-1 text-xs text-slate-400">${this.escape(group.description) || `<span class="italic text-slate-600">${t("groups.no-description")}</span>`}</p>
                <div class="mt-2 flex flex-wrap gap-2 text-[11px]">
                  <span class="text-slate-500">${t("groups.roles")}:</span>
                  ${group.roles.map((r) => `<span class="rounded bg-slate-800 px-2 py-0.5 text-slate-200">${this.escape(r)}</span>`).join("") || `<span class="italic text-slate-600">${t("groups.no-roles")}</span>`}
                </div>
                <div class="mt-1 flex flex-wrap gap-2 text-[11px]">
                  <span class="text-slate-500">${t("groups.scopes")}:</span>
                  ${group.scopes.map((s) => `<span class="rounded bg-slate-800 px-2 py-0.5 text-slate-200">${this.escape(s)}</span>`).join("") || `<span class="italic text-slate-600">${t("groups.no-scopes")}</span>`}
                </div>
              </li>`,
            )
            .join("")}</ul>`;
    return `
      <div class="grid gap-4 lg:grid-cols-[1.4fr_1fr]">
        <div>${list}</div>
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

  private renderAuditModal(): string {
    const rows =
      this.auditEntries.length === 0
        ? `<tr><td colspan="4" class="py-4 text-center text-xs text-slate-500">${t("users.audit.empty")}</td></tr>`
        : this.auditEntries
            .map((entry) => {
              const ts = new Date(entry.timestamp * 1000).toISOString().slice(0, 19).replace("T", " ");
              const outcomeColor = entry.outcome === "ok" ? "text-emerald-300" : "text-amber-300";
              return `<tr class="border-t border-slate-800"><td class="py-1 pr-3 font-mono text-slate-400">${ts}</td><td class="py-1 pr-3 text-cyan-300">${this.escape(entry.action)}</td><td class="py-1 pr-3 ${outcomeColor}">${this.escape(entry.outcome)}</td><td class="py-1 text-slate-300">${this.escape(entry.detail) || "—"}</td></tr>`;
            })
            .join("");
    return `
      <div id="audit-modal-overlay" class="fixed inset-0 z-[80] flex items-center justify-center bg-slate-950/80 backdrop-blur-sm">
        <div class="w-[min(48rem,calc(100vw-2rem))] rounded-lg border border-slate-700 bg-slate-900 p-5">
          <div class="mb-3 flex items-center justify-between">
            <h3 class="text-sm font-semibold text-cyan-300">${t("users.audit.title")} <span class="ml-2 font-mono text-slate-300">${this.escape(this.auditTarget ?? "")}</span></h3>
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
              <tbody>${rows}</tbody>
            </table>
          </div>
        </div>
      </div>
    `;
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

  private escape(value: string): string {
    return value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }
}

customElements.define("tachyon-users-panel", TachyonUsersPanel);
