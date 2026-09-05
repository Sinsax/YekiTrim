import { createRouter, createWebHashHistory } from 'vue-router'
import HomeView from './views/HomeView.vue'
import ProjectsView from './views/ProjectsView.vue'
import ProjectLayout from './views/ProjectLayout.vue'
import ProjectOverviewView from './views/ProjectOverviewView.vue'
import PreviewView from './views/PreviewView.vue'
import ClipsView from './views/ClipsView.vue'
import TranscriptView from './views/TranscriptView.vue'
import PolishView from './views/PolishView.vue'
import ModelsView from './views/ModelsView.vue'
import PolishProjectsView from './views/PolishProjectsView.vue'
import SettingsView from './views/SettingsView.vue'
import JobsView from './views/JobsView.vue'

const routes = [
  { path: '/', name: 'home', component: HomeView },
  { path: '/projects', name: 'projects', component: ProjectsView },
  {
    path: '/project/:cfg',
    name: 'project',
    component: ProjectLayout,
    children: [
      { path: '', redirect: { name: 'project-overview' } },
      { path: 'overview', name: 'project-overview', component: ProjectOverviewView },
      { path: 'preview', name: 'project-preview', component: PreviewView },
      { path: 'clips', name: 'project-clips', component: ClipsView },
      { path: 'transcript', name: 'project-transcript', component: TranscriptView },
      { path: 'polish', name: 'project-polish', component: PolishView }
    ]
  },
  { path: '/polish', name: 'polish', component: PolishProjectsView },
  { path: '/models', name: 'models', component: ModelsView },
  { path: '/settings', name: 'settings', component: SettingsView },
  { path: '/jobs', name: 'jobs', component: JobsView }
]

const router = createRouter({
  history: createWebHashHistory(),
  routes
})

export default router
