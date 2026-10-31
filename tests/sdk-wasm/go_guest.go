package main

import (
	"example.com/tachyon-sdk-audit/gen/tachyon/mesh/handler"
	tachyonsdk "github.com/astorise/Tachyon-Mesh/sdk/go"
	"go.bytecodealliance.org/cm"
)

func init() { handler.Exports.HandleRequest = handle }
func handle(req handler.Request) handler.Response {
	table := tachyonsdk.NewTable("audit-sdk-go")
	defer table.Close()
	if err := table.Set("probe", []byte("tachyon-go-sdk")); err != nil {
		panic(err)
	}
	data, err := table.Get("probe")
	if err != nil {
		panic(err)
	}
	return handler.Response{Status: 200, Body: cm.ToList(data)}
}
func main() {}
