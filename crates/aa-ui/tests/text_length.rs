use aa_ui::UiRuntime;

#[test]
fn native_multiline_and_textbox_length_counts_scalars_and_is_current_in_change_handler() {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.exec("@text_length", r#"
        for _,class in ipairs({'editboxmultiline','textbox'}) do
            local w=UIParent:CreateWidget(class,'length_'..class,'UIParent')
            assert(w:GetTextLength()==0)
            w:SetText('é도😀'); assert(w:GetTextLength()==3)
            w:SetText('é'); assert(w:GetTextLength()==2)
            w.GetText=function() error('must read the native buffer') end
            assert(w:GetTextLength()==2)
            if class=='editboxmultiline' then
                local changes=0
                w:SetHandler('OnTextChanged',function(self)
                    changes=changes+1; assert(self:GetTextLength()==4)
                end)
                w:SetText('AB도😀'); assert(changes==1)
            end
            w:SetHandler('OnTextChanged',nil)
            w:SetText(''); assert(w:GetTextLength()==0)
            local other=UIParent:CreateWidget('label','other_'..class,'UIParent')
            other:SetText('abc')
            assert(select('#',w.GetTextLength(other))==0)
        end
    "#).unwrap();
}
