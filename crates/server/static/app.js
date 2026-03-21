function statusLabel(status) {
  switch (status) {
    case "ready":
      return "Готов";
    case "starting":
      return "Запускается";
    case "error":
      return "Ошибка";
    case "stopped":
      return "Остановлен";
    default:
      return "Неизвестно";
  }
}

async function loadProjects() {
  const projectsNode = document.getElementById("projects");
  const detailsNode = document.getElementById("details");
  projectsNode.innerHTML = "Загрузка...";

  const response = await fetch("/api/projects");
  const projects = await response.json();

  if (!projects.length) {
    projectsNode.innerHTML = "Проектов пока нет.";
    detailsNode.textContent = "Создайте проект через API или основной интерфейс.";
    return;
  }

  projectsNode.innerHTML = "";
  for (const project of projects) {
    const button = document.createElement("button");
    button.className = "project-item";
    button.textContent = `${project.name} [${statusLabel(project.status.status)}]`;
    button.onclick = () => {
      detailsNode.textContent = JSON.stringify(project, null, 2);
    };
    projectsNode.appendChild(button);
  }
}

document.getElementById("reload").addEventListener("click", loadProjects);
loadProjects().catch((error) => {
  document.getElementById("projects").textContent = error.message;
});
