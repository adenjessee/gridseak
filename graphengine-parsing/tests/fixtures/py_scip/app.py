def callee() -> int:
    return 1


def caller() -> int:
    return callee()


class Parser:
    def parse(self) -> str:
        return "ok"


def use_parser(obj: Parser) -> str:
    return obj.parse()
