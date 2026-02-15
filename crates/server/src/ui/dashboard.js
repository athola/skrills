// Skrills Dashboard Client-Side Logic
(function() {
    let skills = [];
    let events = [];
    let selectedSkill = null;

    function escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }

    async function refresh() {
        try {
            const res = await fetch('/api/skills');
            const data = await res.json();
            skills = data.items || [];
            document.getElementById('skill-count').textContent = data.total || skills.length;
            renderSkills();
        } catch (e) {
            console.error('Failed to fetch skills:', e);
        }

        try {
            const res = await fetch('/api/metrics/events');
            const data = await res.json();
            events = data.events || [];
            document.getElementById('event-count').textContent = events.length;
            renderEvents();
        } catch (e) {
            console.error('Failed to fetch events:', e);
        }

        document.getElementById('last-update').textContent = new Date().toLocaleTimeString();
    }

    function createSkillItem(skill) {
        const div = document.createElement('div');
        div.className = 'skill-item';
        div.dataset.name = skill.name;

        const nameSpan = document.createElement('span');
        nameSpan.className = 'skill-name';
        nameSpan.textContent = skill.name;

        const sourceSpan = document.createElement('span');
        sourceSpan.className = 'skill-source';
        sourceSpan.textContent = skill.source;

        div.appendChild(nameSpan);
        div.appendChild(sourceSpan);

        div.addEventListener('click', () => {
            selectedSkill = skill;
            renderMetrics();
        });

        return div;
    }

    function renderSkills() {
        const list = document.getElementById('skill-list');
        list.replaceChildren();

        if (skills.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'empty';
            empty.textContent = 'No skills found';
            list.appendChild(empty);
            return;
        }

        skills.forEach(skill => {
            list.appendChild(createSkillItem(skill));
        });
    }

    function eventDetail(event) {
        if (event.type === 'SkillInvocation') {
            return escapeHtml(event.skill_name) + ' - ' + (event.success ? 'OK' : 'FAIL');
        }
        if (event.type === 'Validation') {
            return escapeHtml(event.skill_name) + ' - ' + (event.checks_failed?.length ? 'FAIL' : 'PASS');
        }
        if (event.type === 'Sync') {
            return escapeHtml(event.operation) + ' - ' + escapeHtml(event.status);
        }
        return JSON.stringify(event).slice(0, 50);
    }

    function createEventItem(event) {
        const div = document.createElement('div');
        div.className = 'activity-item ' + (event.type || '');

        const typeSpan = document.createElement('span');
        typeSpan.className = 'event-type';
        typeSpan.textContent = event.type || 'Event';

        const detailSpan = document.createElement('span');
        detailSpan.className = 'event-detail';
        detailSpan.textContent = eventDetail(event);

        div.appendChild(typeSpan);
        div.appendChild(detailSpan);

        return div;
    }

    function renderEvents() {
        const list = document.getElementById('activity-list');
        list.replaceChildren();

        if (events.length === 0) {
            const empty = document.createElement('div');
            empty.className = 'empty';
            empty.textContent = 'No recent activity';
            list.appendChild(empty);
            return;
        }

        events.slice(0, 50).forEach(event => {
            list.appendChild(createEventItem(event));
        });
    }

    function renderMetrics() {
        const content = document.getElementById('metrics-content');
        content.replaceChildren();

        if (!selectedSkill) {
            const empty = document.createElement('div');
            empty.className = 'empty';
            empty.textContent = 'Select a skill to view details';
            content.appendChild(empty);
            return;
        }

        const h3 = document.createElement('h3');
        h3.textContent = selectedSkill.name;
        content.appendChild(h3);

        const dl = document.createElement('dl');

        const dtPath = document.createElement('dt');
        dtPath.textContent = 'Path';
        const ddPath = document.createElement('dd');
        ddPath.textContent = selectedSkill.path;

        const dtSource = document.createElement('dt');
        dtSource.textContent = 'Source';
        const ddSource = document.createElement('dd');
        ddSource.textContent = selectedSkill.source;

        const dtDesc = document.createElement('dt');
        dtDesc.textContent = 'Description';
        const ddDesc = document.createElement('dd');
        ddDesc.textContent = selectedSkill.description || 'N/A';

        dl.appendChild(dtPath);
        dl.appendChild(ddPath);
        dl.appendChild(dtSource);
        dl.appendChild(ddSource);
        dl.appendChild(dtDesc);
        dl.appendChild(ddDesc);

        content.appendChild(dl);
    }

    // Initialize once
    function init() {
        refresh();
        setInterval(refresh, 30000);
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();
