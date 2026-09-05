import { createRouter, createWebHashHistory } from 'vue-router'
import ProjectLayout from './views/ProjectLayout.vue'

const routes = [
  { path: '/', name: 'home', component: () => import('./views/HomeView.vue') },
  { path: '/projects', redirect: '/' },
  {
    path: '/project/:cfg',
    name: 'project',
    component: ProjectLayout,
    children: [
      { path: '', redirect: { name: 'project-preview' } },
      { path: 'settings', name: 'project-settings', component: () => import('./views/ProjectSettingsView.vue') },
      { path: 'preview', name: 'project-preview', component: () => import('./views/PreviewView.vue') },
      { path: 'slices', name: 'project-slices', component: () => import('./views/ProjectSlicesView.vue') },
      { path: 'slices/:clip', name: 'project-slice-polish', component: () => import('./views/PolishView.vue') },
      { path: 'polish', name: 'project-polish', component: () => import('./views/PolishView.vue') }
    ]
  },
  { path: '/polish', name: 'polish', component: () => import('./views/PolishProjectsView.vue') },
  { path: '/models', name: 'models', component: () => import('./views/ModelsView.vue') },
  { path: '/settings', name: 'settings', component: () => import('./views/SettingsView.vue') },
  { path: '/jobs', name: 'jobs', component: () => import('./views/JobsView.vue') }
]

const router = createRouter({
  history: createWebHashHistory(),
  routes
})

export default router
