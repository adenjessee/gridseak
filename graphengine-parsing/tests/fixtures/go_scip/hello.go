package goscip

func Callee() int { return 1 }

func Caller() int { return Callee() }
