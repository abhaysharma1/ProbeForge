# ProbeForge — Frontend Design System & UI Implementation Specification

**Document type:** Frontend design and implementation contract  
**Project:** ProbeForge  
**Audience:** AI coding agents, frontend developers, UI engineers  
**Priority:** Mandatory  
**Scope:** All frontend pages, components, layouts, interactions, animations, responsive behavior, and visual styling

---

# 1. Purpose and Design Philosophy

ProbeForge is a security testing and backend analysis platform. It allows users to inspect source code for vulnerabilities, assess deployed applications, discover APIs, and perform controlled load testing.

The frontend must feel like a carefully crafted professional software product, not a website generated from a generic AI dashboard template.

The objective is to create a beautiful, minimalist, technically credible interface that combines:

- The visual restraint of premium developer tools.
- The precision of professional security software.
- The clarity of well-designed productivity applications.
- The fluidity of a carefully animated editorial website.
- The information density required by serious engineering workflows.

The website should be visually memorable without relying on excessive decoration.

## 1.1 Core design principles

### Principle 1: Restraint over decoration

Every visual element must have a purpose.

Do not add gradients, glowing backgrounds, floating particles, decorative illustrations, glass panels, or abstract 3D objects simply to make a page appear sophisticated.

Prefer precise typography, strong spacing, subtle borders, carefully selected colors, and intentional composition.

### Principle 2: Function determines appearance

A vulnerability finding, an API endpoint, a test execution, and a scan status should each look different because they represent different kinds of information.

Do not use the same generic card component for every piece of content.

Choose the most appropriate visual structure for the information being presented.

### Principle 3: Minimalism does not mean emptiness

The application should not feel unfinished or excessively sparse.

Use whitespace to establish hierarchy, but provide meaningful content density where users need to inspect findings, understand test results, review request details, or compare performance metrics.

### Principle 4: Motion must communicate

Animations should communicate state changes, hierarchy, continuity, and cause and effect.

A panel opening, scan starting, finding appearing, or chart updating should feel fluid and intentional.

Animation must never delay a critical action or make the application feel sluggish.

### Principle 5: Authenticity over visual trends

Do not imitate whatever happens to be popular in AI-generated websites.

Avoid predictable layouts consisting of a gradient hero, three floating glass cards, a glowing orb, a grid of identical feature cards, and oversized rounded containers.

ProbeForge should have its own visual identity.

---

# 2. Visual Identity

## 2.1 Design direction

The visual direction is:

**Technical editorial minimalism with restrained industrial detailing.**

Imagine a combination of a premium developer application, a high-quality engineering workstation, and a modern security operations interface.

The interface should feel calm, precise, trustworthy, and expensive without looking luxurious for its own sake.

Use a neutral foundation with one distinctive accent color and a small set of semantic colors.

## 2.2 Color palette

Use the following palette as the initial design-system specification. Centralize the values in design tokens so they can be changed consistently.

### Primary light theme

| Token | Value | Purpose |
|---|---|---|
| `background` | `#F7F7F4` | Main application background |
| `surface` | `#FFFFFF` | Primary content surfaces |
| `surface-muted` | `#F0F0EB` | Secondary panels and selected backgrounds |
| `surface-hover` | `#EBEBE5` | Hover states |
| `text-primary` | `#20221F` | Primary text |
| `text-secondary` | `#666961` | Secondary information |
| `text-muted` | `#898C83` | Timestamps and tertiary information |
| `border` | `#E3E4DC` | Default borders |
| `border-strong` | `#CFD1C7` | Emphasized borders |
| `accent` | `#B84A32` | Primary brand accent |
| `accent-hover` | `#983B28` | Accent hover state |
| `accent-soft` | `#F5E9E4` | Subtle accent background |

The warm off-white background and restrained terracotta accent distinguish ProbeForge from the typical blue-and-purple SaaS interface.

The accent should be used selectively for primary actions, selected navigation, active indicators, and a few important highlights. It must not become a background color for large sections of the application.

### Semantic colors

| Token | Value | Meaning |
|---|---|---|
| `severity-critical` | `#B42318` | Critical security finding |
| `severity-high` | `#C24132` | High-severity finding |
| `severity-medium` | `#A96A17` | Medium-severity finding |
| `severity-low` | `#39765B` | Low-severity finding |
| `severity-info` | `#626C76` | Informational finding |
| `status-success` | `#39765B` | Successful operation |
| `status-warning` | `#A96A17` | Warning |
| `status-error` | `#B42318` | Failure |
| `status-running` | `#876A36` | Active operation |

Severity colors must always be paired with readable labels or icons. Never communicate severity through color alone.

Use semantic colors only where their meanings are relevant. Do not turn every chart, button, or decorative element into a colored object.

### Dark theme

If dark mode is implemented, it must be deliberately designed rather than produced by automatically inverting the light theme.

| Token | Value |
|---|---|
| `background` | `#171815` |
| `surface` | `#20211D` |
| `surface-muted` | `#292A25` |
| `surface-hover` | `#32332D` |
| `text-primary` | `#F0F0E9` |
| `text-secondary` | `#B0B2A8` |
| `text-muted` | `#85877E` |
| `border` | `#363730` |
| `border-strong` | `#4A4B43` |
| `accent` | `#D27A60` |
| `accent-hover` | `#E18C72` |

Use the dark theme primarily to improve comfort for extended technical sessions. Maintain contrast, readable code, and distinct severity indicators.

## 2.3 Forbidden color treatments

The following are prohibited as default design choices:

- Neon purple or violet brand palettes.
- Electric blue and cyan gradient combinations.
- Purple-to-blue gradient buttons.
- Glowing borders surrounding cards.
- Bright gradients used as generic page backgrounds.
- Rainbow gradients in charts.
- Saturated multicolor dashboard elements.
- Randomly colored icons for visual variety.

A specific chart or security visualization may require multiple colors, but each color must have a clear semantic or analytical purpose.

---

# 3. Typography System

Typography is one of the primary tools for making the interface feel premium.

Use a restrained type system with distinct roles rather than dozens of arbitrary font sizes.

## 3.1 Font families

Recommended typography:

- **Interface and headings:** Inter, Geist, or an existing high-quality sans-serif already available in the project.
- **Code and technical identifiers:** Geist Mono, JetBrains Mono, or another legible monospace font.

Prefer an existing font setup when it is already appropriate. Avoid downloading multiple font families or introducing unnecessary dependencies.

Do not use futuristic display fonts, sci-fi lettering, or oversized decorative typography.

## 3.2 Type scale

| Role | Size | Weight | Usage |
|---|---:|---:|---|
| Page title | 28–32 px | 600 | Main page heading |
| Section heading | 20–24 px | 600 | Major content sections |
| Subsection heading | 15–18 px | 600 | Panel and card headings |
| Body | 14–15 px | 400 | Main content |
| Secondary body | 13 px | 400 | Supporting information |
| Metadata | 11–12 px | 400–500 | Timestamps and compact labels |
| Button | 13–14 px | 500 | Interactive controls |
| Code | 12–14 px | 400 | Code snippets and identifiers |
| KPI value | 28–36 px | 500–600 | Important metrics |

These values are guidelines. Adjust them for accessibility, responsive layouts, and the amount of information displayed.

## 3.3 Typography rules

1. Use sentence case for headings, buttons, and navigation.
2. Avoid uppercase text for entire headings.
3. Use bold weights sparingly.
4. Avoid making every label bold.
5. Use tabular numerals for metrics and timing information.
6. Use monospace typography for hashes, endpoint paths where appropriate, line numbers, execution IDs, and code.
7. Ensure long identifiers wrap, truncate, or offer a copy action without breaking the layout.
8. Maintain a clear hierarchy between headings, descriptions, labels, and values.

Do not use giant headings throughout the application. The main dashboard is a working environment, not a marketing landing page.

---

# 4. Layout and Spacing System

## 4.1 Spacing scale

Use a consistent spacing scale based on 4-pixel increments.

| Token | Value |
|---|---:|
| `space-1` | 4 px |
| `space-2` | 8 px |
| `space-3` | 12 px |
| `space-4` | 16 px |
| `space-5` | 20 px |
| `space-6` | 24 px |
| `space-8` | 32 px |
| `space-10` | 40 px |
| `space-12` | 48 px |
| `space-16` | 64 px |

Do not introduce arbitrary spacing values throughout individual components unless the layout requires a documented exception.

## 4.2 Application shell

The application should use a persistent sidebar and a flexible main workspace.

Suggested desktop structure:

- Sidebar: 224–248 px.
- Main content: remaining viewport width.
- Page horizontal padding: 28–36 px on large screens.
- Page top padding: 24–32 px.
- Section spacing: 24–32 px.
- Standard control height: 36–40 px.
- Compact table rows: 44–52 px, with taller rows when content requires them.

The main workspace should remain readable on large displays. Avoid excessively wide paragraphs, oversized panels, and charts that occupy the entire screen without adding information.

Use responsive padding on smaller screens.

## 4.3 Border radius

Use a small, consistent radius scale.

- Small controls: 5–7 px.
- Standard panels: 8–10 px.
- Large dialog surfaces: 12 px.
- Pills: fully rounded only when their semantic purpose calls for it.

Avoid the excessive use of 16–32 px rounded containers.

A security tool should feel precise and engineered, not soft, inflated, or toy-like.

## 4.4 Borders and shadows

Prefer subtle borders over shadows.

- Standard panels should generally have a 1 px border.
- Use a shadow only when a component needs to separate from surrounding content, such as a popover or floating dialog.
- Avoid multiple nested shadows.
- Avoid glassmorphism and translucent blur panels.
- Avoid glowing edges and illuminated card outlines.

The visual hierarchy should primarily come from spacing, typography, alignment, and contrast.

---

# 5. Application Navigation

## 5.1 Sidebar structure

The sidebar should be compact, persistent, and organized around the user's work.

Suggested navigation:

**Workspace**
- Overview
- Projects

**Security**
- Code inspection
- Live application scan
- API discovery

**Performance**
- Load testing
- Test history

**Workspace management**
- Findings
- Reports
- Settings

Only expose sections that are implemented and meaningful in the current application. Do not add nonfunctional navigation items just to make the sidebar look complete.

## 5.2 Sidebar design

Requirements:

- Use simple monochrome line icons.
- Use consistent icon sizing, typically 16–18 px.
- Use text labels alongside icons.
- Indicate the active route with a subtle surface background and a clear text treatment.
- Use the terracotta accent sparingly.
- Keep navigation groups visually separated.
- Place account and workspace controls at the bottom where appropriate.
- Support a collapsed mode only if it remains understandable and accessible.

Do not give every navigation item its own bright background, gradient, badge, or decorative icon.

## 5.3 Navigation interactions

- Hover states should appear smoothly.
- Active states should update immediately when navigation changes.
- Preserve appropriate filters and selected project context when moving between related screens.
- Use clear loading states for route transitions that require substantial data fetching.
- Do not animate the entire application shell on every navigation.

---

# 6. Page Design Specifications

Every page must follow the same design system while retaining a layout appropriate to its task.

## 6.1 Overview dashboard

The overview is the user's operational starting point.

### Header

Display:

- Page title.
- A concise explanation of the current workspace.
- A project selector where relevant.
- A primary action to create a scan or start a test.
- Optional time range or date filters when they affect displayed metrics.

Avoid large marketing-style welcome banners.

### Key metrics

Display a small set of meaningful metrics, such as:

- Projects monitored.
- Completed scans.
- Open findings.
- Critical and high-severity findings.
- Recent load-test status or performance trends.

Only display metrics supported by actual backend data.

Use compact metric blocks with a clear label, a readable value, and optional trend context.

Do not use a large collection of identical colorful KPI cards.

### Main content

Prefer a useful operational layout containing:

1. Recent scans and test executions.
2. Findings that require attention.
3. Recent project activity.
4. A compact performance trend or scan history visualization, where useful.

Use tables or structured lists for operational data. Use charts only when they reveal trends or comparisons that are difficult to understand from raw numbers.

### Empty state

When no projects or scans exist, explain what the user should do next and provide one clear action.

Do not populate the dashboard with fake statistics or artificial activity.

## 6.2 Project management

A project represents a codebase or deployed application being assessed.

### Project list

Use a clean table or structured list containing:

- Project name.
- Project type.
- Repository or target reference, where appropriate.
- Last scan time.
- Current status.
- Open findings summary.
- Available actions.

Keep metadata compact.

Use subtle status indicators and text labels rather than colorful status cards.

### Create project flow

Provide a focused, multi-step flow when the setup requires multiple independent decisions.

Possible steps:

1. Select the project type.
2. Configure the repository or target.
3. Review the scope and required permissions.
4. Validate the configuration.
5. Create the project.

The flow must reflect the actual backend capabilities. Do not invent integrations or claim a target is verified when it has not been checked.

Show validation errors beside the affected fields.

Use clear descriptions for security-sensitive configuration.

## 6.3 Source-code inspection

This screen analyzes a repository or uploaded codebase for potential vulnerabilities.

### Primary layout

Use a structured workspace with:

- Project and scan information at the top.
- Scan progress and current status.
- A findings table or list.
- Filters for severity, status, category, and affected file.
- A detail panel for the selected finding.

The findings list should be the primary element. Avoid surrounding it with unnecessary decorative cards.

### Finding list

Each finding should show:

- Severity.
- Finding title.
- Affected file or component.
- Relevant line number, where available.
- Rule or vulnerability category.
- Confidence or verification status, when available.
- Workflow status.

Use compact rows with clear alignment.

The severity indicator should be visible but not overpower the finding title.

### Finding details

On selecting a finding, show:

- Finding title and severity.
- Explanation of the vulnerability.
- Evidence supporting the finding.
- Affected file and line range.
- Relevant source-code excerpt.
- Potential impact.
- Remediation guidance.
- References or rule identifiers, when available.
- Verification and triage status.

Use a side panel or dedicated detail route depending on the amount of information.

Do not present a potential vulnerability as a confirmed exploit unless the available evidence supports that claim.

### Source-code presentation

Use a monospace font, subtle line-number column, readable syntax highlighting, and restrained selection colors.

Highlight only the relevant lines.

Avoid neon syntax themes, unnecessary animated code effects, and glowing source-code panels.

## 6.4 Live application security scan

This screen is used to assess a deployed application within an explicitly authorized scope.

### Configuration interface

Present target configuration in a clear form with:

- Target origin or base URL.
- Approved scope.
- Scan profile.
- Optional request limits and timeouts, if supported.
- Authentication configuration, if supported.
- A clear summary of the operation.
- An explicit start action.

Make authorization and scope boundaries visible before the scan starts.

Do not make the primary action appear harmless if it can generate substantial traffic or affect a production service.

### Scan progress

Display meaningful progress information, such as:

- Current scan status.
- Current phase.
- Requests completed, when available.
- Findings discovered.
- Elapsed time.
- Errors or blocked operations.
- Cancellation action, if supported.

Use a progress bar only when the system can provide a meaningful progress value.

For indeterminate work, use a subtle animated indicator and clear status text.

Do not invent percentage completion.

### Results

Use a structured findings view consistent with the source-code inspection page.

Separate discovery, verification, and remediation status when the backend exposes these distinctions.

## 6.5 API discovery

This page presents endpoints discovered from a repository or authorized application analysis.

### Endpoint explorer

Use a table or list with:

- HTTP method.
- Endpoint path.
- Short description, when available.
- Authentication requirement, when known.
- Discovery source.
- Testing status.
- Associated findings, where relevant.

Use compact method labels with restrained semantic styling.

Do not assign random colors to HTTP methods.

### Endpoint detail

Show:

- Full path and method.
- Parameters and request schema.
- Request body schema.
- Response schema and status codes.
- Authentication requirements.
- Source location, when discovered from code.
- Associated security findings.
- Test results and timestamps, where available.

Provide copy actions for useful technical identifiers.

For long endpoint paths or schemas, support scrolling and wrapping without breaking the page.

## 6.6 Load testing

This screen allows users to configure and inspect performance tests.

### Configuration

Provide clearly grouped fields for:

- Target URL or approved endpoint.
- HTTP method.
- Request headers and payload, where applicable.
- Concurrency or virtual-user count.
- Test duration or total iterations.
- Ramp-up behavior, if supported.
- Request timeout.
- Other supported execution limits.

Use numeric inputs with validation and clear units.

Explain potentially expensive settings. Display relevant warnings before running tests against a production target.

Require confirmation for high-impact configurations when appropriate.

### Test execution

Show:

- Current execution status.
- Elapsed time.
- Completed requests.
- Successful and failed requests.
- Current throughput.
- Response latency metrics.
- Errors and timeouts.
- Cancellation controls, where supported.

Update values from real execution events or polling data. Avoid artificially incrementing counters or fabricating live traffic.

### Results

Present useful performance metrics:

- Requests per second.
- Total requests.
- Success and failure counts.
- Error rate.
- Median latency.
- p90 and p95 latency.
- p99 latency when available.
- Timeout count.
- Throughput over time.

Use a small number of legible charts.

A latency distribution or time-series chart should have clear axes, units, readable labels, and tooltips.

Do not use a pie chart for every metric.

Keep raw results and error details accessible below or beside the summary.

## 6.7 Findings management

Create a unified view of findings across supported scan types.

Useful capabilities include:

- Search.
- Severity filters.
- Finding category filters.
- Project filters.
- Status filters.
- Sorting.
- Finding detail access.
- Remediation status.
- Export actions where supported.

Use a dense but readable table.

Allow users to distinguish new findings, recurring findings, verified issues, accepted risks, and resolved findings where those states exist in the backend.

Do not conflate the severity of a vulnerability with the confidence of its detection.

## 6.8 Reports

The reports page should present generated reports with:

- Report title.
- Project.
- Report type.
- Generation date.
- Status.
- Relevant summary.
- Supported export or download action.

Use a simple table or list. Avoid a wall of report cards.

Show a progress or error state when report generation is asynchronous.

## 6.9 Settings

Organize settings into logical sections:

- Workspace.
- Projects and integrations.
- Scan defaults.
- Test defaults.
- Appearance.
- Account and access controls, if supported.

Avoid a single excessively long settings page.

Use clear save, cancel, success, and error states. Destructive actions require an appropriate confirmation flow.

---

# 7. Component Design System

Create reusable components before implementing every page independently.

The design system should be consistent without forcing unrelated interactions into the same component.

## 7.1 Core components

Implement components appropriate to the project architecture, including:

- `AppShell`
- `Sidebar`
- `TopBar`
- `PageHeader`
- `Button`
- `IconButton`
- `Input`
- `Select`
- `Checkbox`
- `Switch`
- `Tooltip`
- `Popover`
- `Dialog`
- `DropdownMenu`
- `Tabs`
- `Badge`
- `StatusIndicator`
- `SeverityIndicator`
- `DataTable`
- `FilterBar`
- `SearchInput`
- `MetricDisplay`
- `EmptyState`
- `Skeleton`
- `ProgressIndicator`
- `CodeViewer`
- `FindingList`
- `FindingDetailPanel`
- `EndpointTable`
- `LatencyChart`
- `Toast`
- `ConfirmationDialog`

Names are illustrative. Reuse existing components when they already provide equivalent functionality.

## 7.2 Buttons

Establish a small button hierarchy:

**Primary**
- Solid terracotta background.
- White text.
- Reserved for the main action in a section.

**Secondary**
- Neutral surface.
- Subtle border.
- Used for ordinary supporting actions.

**Ghost**
- Transparent background.
- Used for low-emphasis actions.

**Destructive**
- Uses semantic error styling.
- Reserved for destructive operations.

Requirements:

- Clear hover, focus, disabled, and loading states.
- Consistent control heights.
- Visible keyboard focus.
- No unnecessary gradients.
- No oversized pill-shaped buttons by default.
- No excessive hover movement.

A page should generally have one visually dominant primary action per workflow context.

## 7.3 Cards and panels

A card should exist because it creates a useful grouping.

Use panels for:

- Complex forms.
- Finding details.
- Code excerpts.
- Performance results.
- Clearly separated work areas.

Avoid wrapping every heading, metric, label, and paragraph in its own card.

Where a table can communicate information more clearly, prefer the table.

## 7.4 Tables

Tables are essential to ProbeForge.

They must support:

- Consistent column alignment.
- Clear headers.
- Compact rows.
- Subtle separators.
- Hover states.
- Sorting where useful.
- Filtering where useful.
- Loading and empty states.
- Keyboard accessibility.
- Responsive overflow behavior.

Do not use heavy borders around every cell.

Avoid excessive whitespace between columns that causes important technical information to disappear below the fold.

## 7.5 Icons

Use one consistent icon family, preferably Lucide or an equivalent existing library.

Rules:

- Prefer line icons.
- Keep sizes consistent.
- Use icons to support labels rather than replace understandable text.
- Do not mix outline, filled, emoji, and 3D icons.
- Avoid decorative icons without functional meaning.

Use tooltips for icon-only actions.

---

# 8. Animation and Motion System

Animation is an important part of the visual identity, but it must remain subtle and purposeful.

Use the project's existing animation library when possible. If a library is not already present, choose one appropriate library rather than introducing several overlapping animation systems.

For a React application, CSS transitions and keyframes should handle simple interactions. Use Framer Motion or Motion for coordinated component transitions and layout animations when they provide meaningful value.

Use GSAP only when a genuinely complex, timeline-based interaction justifies it.

Do not add a heavy animation library solely to animate buttons and cards.

## 8.1 Motion timing

Use the following starting values:

| Interaction | Duration |
|---|---:|
| Button hover | 100–160 ms |
| Input focus | 120–180 ms |
| Tooltip appearance | 120–180 ms |
| Dropdown opening | 140–200 ms |
| Dialog appearance | 180–240 ms |
| Side panel transition | 200–280 ms |
| Page content transition | 180–260 ms |
| Chart updates | 250–450 ms |
| Complex introductory sequence | 400–700 ms |

These values are guidelines, not mandatory delays. Avoid adding artificial waits to data-dependent actions.

Use a consistent easing curve, such as an ease-out curve for entrances and a standard ease-in-out curve for reversible state changes.

## 8.2 Required interactions

### Buttons

- Transition background and border colors smoothly.
- Use little or no movement.
- Avoid dramatic scaling.
- Do not use glow effects.

### Dropdowns

- Fade and translate slightly into place.
- Keep the animation short.
- Ensure the menu is immediately usable.

### Dialogs

- Use a subtle opacity transition and a small transform.
- Use a restrained backdrop.
- Avoid oversized spring animations.

### Side panels

- Slide in smoothly from the appropriate edge.
- Keep the underlying layout stable where possible.
- Preserve the selected item when the panel updates.

### Lists

- New results may fade into view.
- Use staggered entrances only for small, intentional groups.
- Never animate hundreds of table rows individually.

### Charts

- Animate meaningful data changes without repeatedly restarting the entire chart.
- Keep tooltips responsive.
- Avoid decorative continuous motion.

### Loading states

Use skeletons for content whose structure is known.

Use a small spinner or indeterminate indicator for actions that complete relatively quickly or do not have a meaningful progress measure.

Use actual progress only when the backend supplies sufficient information.

## 8.3 Avoid excessive motion

Do not:

- Animate the entire page on every interaction.
- Use perpetual floating objects.
- Use constant background movement.
- Add animated particles or network meshes.
- Apply parallax to ordinary dashboard content.
- Animate every metric continuously.
- Delay critical actions until an animation finishes.
- Use spring effects on every component.
- Make data tables bounce or slide unnecessarily.

## 8.4 Reduced motion

Respect `prefers-reduced-motion`.

When reduced motion is enabled:

- Remove unnecessary transforms and staggered entrances.
- Reduce or remove decorative motion.
- Keep essential state transitions understandable.
- Never depend on animation to communicate status.

---

# 9. Backgrounds, Textures, and Visual Effects

The default application background should be a solid warm off-white.

Subtle visual texture may be used only when it improves the design.

Permitted treatments:

- A restrained neutral background for secondary surfaces.
- Very subtle separators.
- Carefully selected monochrome diagrams when they explain architecture.
- Simple charts that visualize actual data.
- Restrained accent-colored selection indicators.
- Subtle background differences between the navigation and workspace.

Avoid:

- Mesh gradients.
- Animated gradient backgrounds.
- Blurred colored orbs.
- Glassmorphism.
- Neon outlines.
- Fake terminal windows used as decoration.
- Random technical symbols in the background.
- Artificial scanlines.
- Glowing circuit-board patterns.
- 3D objects that do not communicate real application functionality.
- Heavy noise overlays.
- Excessive drop shadows.

## 9.1 Shaders and WebGL

WebGL, Three.js, custom shaders, and GPU-heavy effects are not part of the default design system.

Use them only if a clearly defined interaction genuinely benefits from them and the implementation can be kept accessible and performant.

A shader must never be used simply because the website is supposed to look modern.

For ProbeForge's main dashboard and operational screens, good typography, precise spacing, clear information hierarchy, and smooth transitions will produce a better result.

---

# 10. Data Visualization

Charts must serve analytical tasks rather than decorate the dashboard.

## 10.1 Chart rules

- Use neutral axes and gridlines.
- Use one primary series color where possible.
- Use semantic colors for meaningful thresholds or error states.
- Use readable labels and units.
- Provide tooltips for detailed values.
- Handle missing or incomplete data explicitly.
- Keep chart animations restrained.
- Ensure values are not communicated through color alone.

## 10.2 Recommended chart types

| Information | Preferred visualization |
|---|---|
| Requests per second over time | Line chart |
| Latency over time | Line chart |
| Latency distribution | Histogram or distribution chart |
| Error rate over time | Line chart |
| Findings by severity | Compact stacked bar or segmented summary |
| Scan history | Timeline or structured list |
| Individual endpoint performance | Table or ranked list |

Do not create charts for values that are more easily compared as text.

Do not display invented historical trends when the platform has insufficient execution history.

---

# 11. Responsive Design

The application must work across desktop, tablet, and mobile screens.

## 11.1 Desktop

At widths above approximately 1200 px:

- Use the persistent sidebar.
- Display tables with their primary columns.
- Use side panels for contextual details where useful.
- Allow charts and technical panels to sit side by side when there is sufficient space.

## 11.2 Tablet

At widths between approximately 768 px and 1200 px:

- Reduce page padding.
- Collapse or compact the sidebar.
- Reduce nonessential table columns.
- Allow secondary panels to move below the primary content.
- Maintain usable filter and action controls.

## 11.3 Mobile

Below approximately 768 px:

- Use a compact navigation drawer or another appropriate mobile navigation pattern.
- Stack major sections vertically.
- Keep primary actions accessible.
- Allow tables to scroll horizontally or switch to a carefully designed compact list.
- Ensure dialogs fit within the viewport.
- Make forms and filters usable with touch.
- Prevent long paths, code snippets, and identifiers from causing page overflow.

Do not merely shrink the desktop interface until it fits.

The mobile layout must preserve the most important tasks and information.

---

# 12. Accessibility and Interaction Quality

Accessibility is a core implementation requirement.

The frontend must include:

- Semantic HTML.
- Correct heading hierarchy.
- Keyboard navigation.
- Visible focus states.
- Accessible form labels.
- Appropriate ARIA attributes where needed.
- Sufficient text and control contrast.
- Accessible dialogs and menus.
- Text labels for status and severity.
- Reduced-motion support.
- Useful error messages.
- Screen-reader-friendly status updates for asynchronous operations.

Do not use clickable `div` elements when a button or link is appropriate.

Do not remove focus outlines without replacing them with an equally visible focus treatment.

All critical operations must remain usable without a mouse.

---

# 13. Loading, Empty, Error, and Success States

Every data-dependent screen must handle the full lifecycle of a request.

## 13.1 Loading

Use skeletons that resemble the final content structure.

Avoid blank screens, excessive spinners, and arbitrary simulated loading delays.

## 13.2 Empty

Explain why there is no data and what the user can do next.

Examples:

- No projects yet: provide the create-project action.
- No findings: explain that no findings are currently available for the selected scope.
- No scan history: offer the appropriate first scan action.
- No load-test results: explain that a test must be completed before results can be displayed.

Do not claim that a project is secure simply because a scan has not been run.

## 13.3 Error

Show concise, actionable errors.

Where useful, offer a retry action and preserve valid user input.

Do not expose stack traces, secrets, credentials, or internal infrastructure details to end users.

## 13.4 Success

Confirm the action clearly without unnecessary celebration animations.

Use inline status messages, a toast, or a dedicated success state depending on the importance of the operation.

Do not show a success message until the corresponding operation has actually succeeded.

---

# 14. Technical Implementation Rules

The frontend must implement the design system using the existing project architecture.

## 14.1 Before implementation

The coding agent must:

1. Inspect the existing repository.
2. Identify the frontend framework and build tooling.
3. Inspect existing routing, components, styling, and dependency versions.
4. Identify existing design-system components that can be reused.
5. Review API contracts and actual backend capabilities.
6. Establish the shared design tokens.
7. Implement the application shell and foundational components.
8. Implement the pages using the shared system.

Do not replace a functioning architecture or reinstall the frontend from scratch simply to satisfy visual preferences.

## 14.2 Styling

Use centralized design tokens for:

- Colors.
- Typography.
- Spacing.
- Border radii.
- Borders.
- Shadows.
- Motion timing.
- Z-index layers.
- Responsive breakpoints.

Use the project's established styling system. If Tailwind is already used, implement these tokens through its supported configuration or CSS variable approach.

Avoid arbitrary inline styles scattered throughout the codebase.

Do not create a separate, inconsistent palette for every page.

## 14.3 Component architecture

Separate:

- Application shell and navigation.
- Page layouts.
- Reusable interface components.
- Domain-specific security components.
- Data fetching and API integration.
- Form validation.
- State management.
- Animation behavior.

Avoid giant components that contain an entire page, all its data-fetching logic, all styling decisions, and every interaction.

Avoid creating abstractions that add complexity without improving reuse or clarity.

## 14.4 Backend integration

The frontend must use real backend contracts wherever they are available.

Rules:

- Do not invent API endpoints.
- Do not hardcode fake scan results into production screens.
- Do not fabricate progress percentages.
- Do not simulate successful scans when the backend has not completed them.
- Do not silently swallow request failures.
- Do not expose secrets in client-side code.
- Keep credentials and sensitive server operations on the appropriate backend.
- Provide explicit mock data only in clearly identified development fixtures or stories.

Use consistent loading, error, and empty states for every integration.

## 14.5 Performance

- Avoid unnecessary re-renders.
- Lazy-load large routes and heavy visualizations when appropriate.
- Keep dependencies minimal.
- Avoid unnecessary icon packages.
- Virtualize genuinely large tables when profiling shows it is necessary.
- Clean up event listeners and animation resources.
- Avoid continuous GPU-heavy animations.
- Preserve responsiveness while scans and tests are running.

Do not optimize prematurely at the cost of readability.

---

# 15. Frontend Quality Gates

A page is not complete simply because it renders.

Every implemented screen must be reviewed against the following criteria.

## Visual quality

- Does the page follow the established palette?
- Is the typography hierarchy consistent?
- Is the spacing intentional?
- Are borders and shadows restrained?
- Are components aligned correctly?
- Does the page avoid generic AI-dashboard styling?
- Is the interface visually coherent with the rest of ProbeForge?

## Interaction quality

- Do all buttons and links work?
- Are loading, disabled, success, and error states present?
- Are filters and search functional?
- Are dialogs and menus keyboard-accessible?
- Do transitions behave correctly?
- Can users understand the result of their actions?

## Technical quality

- Are real backend responses handled correctly?
- Are errors handled gracefully?
- Are long strings and large datasets supported?
- Does the layout work on smaller screens?
- Are console errors and avoidable warnings resolved?
- Are animations cleaned up correctly?

## Accessibility quality

- Can the page be navigated with a keyboard?
- Are controls labeled?
- Is focus visible?
- Is contrast sufficient?
- Is reduced motion respected?
- Are statuses understandable without color?

A page must pass all relevant quality gates before it is considered complete.

---

# 16. Mandatory Design Anti-Patterns

The following are explicit rejection criteria.

An implementation must be revised if it relies on:

1. Blue-purple gradient backgrounds as its primary visual identity.
2. Neon accents and glowing outlines across the interface.
3. Glassmorphism as the default surface style.
4. Excessively rounded cards and controls.
5. The same three-card layout on every page.
6. Oversized marketing headings inside operational screens.
7. Emoji as a substitute for a consistent icon system.
8. Decorative charts containing meaningless data.
9. Fake terminal interfaces that do not perform real work.
10. Animated particles or floating 3D objects without a functional purpose.
11. Excessive animation on every component.
12. Inconsistent colors and spacing between pages.
13. Empty white space caused by incomplete implementation.
14. Fake metrics, fake activity, or fabricated scan progress.
15. Unreadable tables caused by over-prioritizing visual minimalism.
16. A generic admin template with only its colors changed.
17. Excessive shadows, gradients, or nested containers.
18. Nonfunctional controls presented as finished features.

The agent must correct these issues rather than justify them as stylistic choices.

---

# 17. Implementation Sequence

Follow this sequence when building the frontend from scratch.

### Stage 1: Design foundations

- Establish color, spacing, typography, radius, and motion tokens.
- Configure global styles.
- Establish light and dark theme behavior if dark mode is in scope.
- Define responsive breakpoints.
- Establish icon conventions.

**Completion condition:** A consistent design foundation exists and can be reused throughout the application.

### Stage 2: Application shell

- Implement the sidebar.
- Implement the top bar and workspace controls.
- Implement the page container.
- Implement navigation and active states.
- Verify responsive behavior.

**Completion condition:** Every implemented route can use the same application shell.

### Stage 3: Core components

- Build buttons, inputs, menus, dialogs, badges, tables, filters, and feedback states.
- Add keyboard interaction.
- Implement reduced-motion behavior.
- Document component variants.

**Completion condition:** Pages can be assembled from consistent, functional primitives rather than ad hoc styling.

### Stage 4: Operational pages

Implement:

1. Overview dashboard.
2. Projects.
3. Source-code inspection.
4. Live application scan.
5. API discovery.
6. Load testing.
7. Findings management.
8. Reports and settings.

Adjust the order to match actual backend readiness, but do not implement a page using invented capabilities.

**Completion condition:** Every in-scope page is usable, integrated with available APIs, and visually consistent.

### Stage 5: Motion and polish

- Refine hover and focus states.
- Add appropriate panel and dialog transitions.
- Animate supported chart updates.
- Verify loading and empty-state transitions.
- Remove unnecessary visual effects.

**Completion condition:** Motion improves clarity without reducing responsiveness.

### Stage 6: Quality assurance

- Test desktop, tablet, and mobile layouts.
- Test keyboard navigation.
- Test loading, error, empty, and success states.
- Inspect long paths, large tables, and technical content.
- Check contrast and reduced-motion behavior.
- Review every page for forbidden design patterns.
- Resolve obvious visual inconsistencies.

**Completion condition:** The frontend meets the quality gates in this document.

---

# 18. Final Instruction to the Coding Agent

Treat this document as the authoritative frontend design contract for ProbeForge.

Before adding a visual effect, ask whether it improves comprehension, interaction, hierarchy, or usability. If it does not, leave it out.

Before creating a new component, check whether an existing component can be extended without compromising its clarity.

Before using an animation, determine what state transition it communicates and whether the animation is necessary.

Before presenting a metric, confirm that the data exists and that its meaning is clear.

Before declaring a page finished, inspect it at realistic viewport sizes and test its actual interactions.

The desired result is not a flashy security-themed website. It is a refined, dependable engineering product with a recognizable visual identity.

**The design should feel intentional, not generated.**

Prioritize:

1. Clarity over decoration.
2. Consistency over novelty.
3. Useful information over visual noise.
4. Subtle motion over spectacle.
5. Real functionality over simulated interactions.
6. Readability over excessive minimalism.
7. Thoughtful details over generic trends.

When a design decision is uncertain, choose the simpler, more legible, more restrained solution that best supports the user's task.
