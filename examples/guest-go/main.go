package main

import (
	"fmt"
	"io"
	"os"
)

func main() {
	_, _ = io.ReadAll(os.Stdin)
	fmt.Print("Hello from TinyGo FaaS!")
}
