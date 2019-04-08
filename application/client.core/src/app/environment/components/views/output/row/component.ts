import { Component, Input, AfterContentChecked } from '@angular/core';
import { DomSanitizer, SafeHtml } from '@angular/platform-browser';
import { IStreamPacket } from '../../../../controller/controller.session.tab.stream.output';
import PluginsService, { IPluginData } from '../../../../services/service.plugins';

@Component({
    selector: 'app-views-output-row',
    templateUrl: './template.html',
    styleUrls: ['./styles.less']
})

export class ViewOutputRowComponent implements AfterContentChecked {
    @Input() public row: IStreamPacket | undefined;

    public _ng_safeHtml: SafeHtml = null;
    public _ng_sourceName: string | undefined;
    public _ng_number: string | undefined;
    public _ng_pending: boolean | undefined;

    constructor(private _sanitizer: DomSanitizer) {
    }

    ngAfterContentChecked() {
        if (this.row.position.toString() === this._ng_number) {
            return;
        }
        if (this.row.pending) {
            this._acceptPendingRow();
        } else {
            this._acceptRowWithContent();
        }
    }

    private _acceptRowWithContent() {
        if (this.row.pluginId === -1) {
            return;
        }
        const plugin: IPluginData | undefined = PluginsService.getPluginById(this.row.pluginId);
        let html = this.row.str;
        if (plugin === undefined) {
            this._ng_sourceName = 'n/d';
        } else {
            this._ng_sourceName = plugin.name;
            if (plugin.parsers.row !== undefined) {
                html = plugin.parsers.row(html);
            }
        }
        this._ng_safeHtml = this._sanitizer.bypassSecurityTrustHtml(html);
        this._ng_number = this.row.position.toString();
        this._ng_pending = false;
    }

    private _acceptPendingRow() {
        this._ng_pending = true;
        this._ng_number = this.row.position.toString();
    }

}
