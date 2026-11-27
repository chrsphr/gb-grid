// ECharts, tree-shaken to the chart types and components this app uses.
import * as echarts from 'echarts/core';
import { HeatmapChart, LineChart, ScatterChart } from 'echarts/charts';
import {
	AxisPointerComponent,
	CalendarComponent,
	DataZoomComponent,
	GridComponent,
	LegendComponent,
	MarkLineComponent,
	ToolboxComponent,
	TooltipComponent,
	VisualMapComponent
} from 'echarts/components';
import { CanvasRenderer } from 'echarts/renderers';

echarts.use([
	HeatmapChart,
	LineChart,
	ScatterChart,
	AxisPointerComponent,
	CalendarComponent,
	DataZoomComponent,
	GridComponent,
	LegendComponent,
	MarkLineComponent,
	ToolboxComponent,
	TooltipComponent,
	VisualMapComponent,
	CanvasRenderer
]);

export { echarts };
export type { EChartsCoreOption as Option } from 'echarts/core';
