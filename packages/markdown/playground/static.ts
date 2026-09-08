import { mount } from 'svelte';
import StaticPreview from './StaticPreview.svelte';
import './preview.css';

mount(StaticPreview, { target: document.getElementById('app')! });
