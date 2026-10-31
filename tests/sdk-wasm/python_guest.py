from wit_world.exports import Handler as HandlerInterface
from wit_world.exports.handler import Response
from tachyon_mesh_sdk import Table

class Handler(HandlerInterface):
    def handle_request(self, req):
        with Table("audit-sdk-python") as table:
            table.set("probe", b"tachyon-python-sdk")
            value = table.get("probe")
        return Response(status=200, headers=[], body=value, trailers=[])
