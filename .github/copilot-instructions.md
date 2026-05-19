# Copilot Instructions — CRM API (.NET Microservices)

Never end the conversation without being told, through the ask-user-question tool, that the user is done. After each action, use the ask user question tool to prompt the user for the next step. Always wait for the user's response before taking another action. Before committing code, always ask the user to review the changes and confirm that you should proceed. If the user says no, ask them what changes they would like to see and make those changes before asking them to review again. After committing, ask what is next. 

*Remember, always use the ask user question tool when you finish working.*

## Build, Test, and Run

```bash
# Restore & build the entire solution
cd API && dotnet restore CRMAPI.sln && dotnet build CRMAPI.sln

# Build a single microservice
dotnet build TestDomain.Pmrt/TestDomain.Pmrt.csproj

# Run all tests
dotnet test CRMAPI.sln

# Run a single test project
dotnet test TestDomain.Pmrt.Tests/TestDomain.Pmrt.Tests.csproj

# Run a single test by name
dotnet test --filter "FullyQualifiedName~GetPmrtData_ValidInput_ReturnsPrograms"

# Code coverage (PowerShell)
cd .. && ./coverage.ps1
```

NuGet packages are sourced from a private Azure DevOps feed configured in `Nuget.config`. The solution targets **.NET 8**. You cannot run NuGet restore or build without access to the package feed, which you don't have. However, you can still read and edit the code, and run tests that don't require building (e.g., if they only reference already-built assemblies). Note that the gateway files and graphql schema will always be regenerated on build, but you won't see those changes, so you should assume any problem is unrelated to their being outdated. 

## Architecture

This is a **REST-first microservice platform** with **GraphQL federation**. Each microservice owns a data source and exposes both REST and GraphQL APIs. A central **Gateway** composes all subgraph schemas via HotChocolate Fusion.

```
Client → REST Controller → ElementService.G2RElement() → Gateway (GraphQL) → Subgraph Query → Service → Data Source
                                                                                                  ↕
                                                                                             SQL Cache
```

### Project layout

| Layer | Projects | Purpose |
|-------|----------|---------|
| **Orchestration** | `CRMAPI.AppHost` | .NET Aspire host — registers all services, connection strings, gateway |
| **Gateway** | `TestDomain.Gateway` | HotChocolate Fusion gateway — composes `gateway.fgp` from subgraphs |
| **Shared defaults** | `CRMAPI.ServiceDefaults` | Serilog, OpenTelemetry, health checks, resilience |
| **Common libraries** | `TestDomain.Commons`, `.Commons.Sql`, `.Commons.Vantage` | Base classes, ElementService, caching, attributes |
| **Microservices** | `TestDomain.{Pmrt,Adss,Twc,Dsm,Tears,Sam,Gfebs,...}` | One per data source |
| **Tests** | `TestDomain.{Service}.Tests`, `TestDomain.Tests` | Unit + integration tests |

### Three data-access patterns

Every microservice follows one of these patterns:

1. **SQL-backed** (Tears, DSM, Sam, AIMMS, F2S, Te, SBA): Inherits `SqlService<TDbContext, TElementOutput>` → EF Core → SQL Server. Name filtering via `IQueryable.Where()` before pagination.

2. **Vantage-backed** (GFEBS, Equipment, UnitHierarchy): Uses `VantageService` → Vantage REST API → SQL cache (500-record chunks). Filtering via `Func<T, bool> filter` parameter.

3. **Custom API-backed** (PMRT, ADSS, Dave): Custom service implementation → external REST/OData API → mapping → SQL cache. In-memory filtering after retrieval, before pagination.

## Key Conventions

### Model hierarchy

All response models inherit from `ElementOutput(string Id, string Name, Guid? SourceId)`. Responses are always wrapped in `ElementOutputData<T>` which includes `Data` (immutable list) and `Meta.Count` (total record count for pagination).

### REST → GraphQL bridge (G2RElement)

Controllers don't call services directly for list endpoints. Instead, they call `IElementService.G2RElement<T>()` which dynamically builds a GraphQL query from the model's properties, executes it against the Gateway, and returns typed results. The pattern:

```csharp
var queryParams = ImmutableDictionary.Create<string, string>()
    .Add("$pageSkip", "Int!")
    .Add("$pageSize", "Int!");

var queryValues = ImmutableDictionary.Create<string, object?>()
    .Add("pageSkip", pageSkip)
    .Add("pageSize", pageSize);

return Ok(await _elementService.G2RElement<YourModel>(
    queryName: "yourElements(pageSkip: $pageSkip, pageSize: $pageSize)",
    queryParams: queryParams,
    queryValues: queryValues));
```

- `queryParams`: GraphQL variable declarations (keys prefixed with `$`, values are GraphQL types)
- `queryValues`: Actual parameter values (keys without `$`)
- `queryName`: The GraphQL field name with parameter bindings
- The query body (field selections) is auto-generated from `T`'s properties via reflection

### GraphQL query classes

Each microservice has a `Queries/` folder with `[QueryType]` classes that define the GraphQL schema. Method names become GraphQL field names (PascalCase → camelCase). These classes call the service layer directly (not through ElementService).

```csharp
[QueryType]
public class YourQuery
{
    public async Task<IEnumerable<YourModel>> GetYourElements(
        [Service] IYourService service, int pageSkip, int pageSize)
        => await service.GetData(pageSkip, pageSize);
}
```

### Federation & subgraph registration

Each microservice must:
1. Have a `subgraph-config.json` with the service name and GraphQL endpoint
2. Expose a `SourceId` query (e.g., `public Guid Pmrt() => settings.SourceId`)
3. Be registered in `AppHost/Program.cs` with `.WithSubgraph()` and `.WithReference()`

### Custom attributes

- `[NameFilterable]` — marks REST endpoints that support `?name=` filtering
- `[RGIgnore]` — excludes model properties from auto-generated GraphQL query body
- `[DevOnlyActionFilterAttribute]` — blocks non-GET requests in production

### Enum handling

GraphQL enums use SCREAMING_SNAKE_CASE. Use `EnumExtensions.ToGraphQLEnumString()` to convert .NET enums (e.g., `InactiveOpen` → `INACTIVE_OPEN`). REST APIs use `[EnumMember]` attribute values via `ToEnumMemberString()`.

### Caching

SQL-based caching with versioned keys. `VersionHelper.GenerateCacheKey(version, typeName)` generates cache keys. `CacheCountHelper` tracks total counts. Cache TTL is typically 30 minutes. Background cache updates (`_ = UpdateCacheInBackgroundAsync(...)`) prevent blocking user responses.

### Error handling in ElementService

`G2RElement` catches exceptions and returns `ElementOutputData<T>(ImmutableList<T>.Empty, -1)` — an empty list with count `-1` signals an error occurred. Callers should check for empty data and log warnings.

### Controller conventions

- All controllers use `[ApiVersion("3.0")]` (current version)
- Route pattern: `api/v{version:apiVersion}/[controller]`
- Pagination: `pageSkip` (0-indexed page number) and `pageSize` (items per page, max 5000)
- Always return `Ok(result)` — pagination metadata is in `ElementOutputData.Meta`

### Testing conventions

- **Framework**: MSTest (`[TestClass]`, `[TestMethod]`, `[TestInitialize]`)
- **Mocking**: Moq
- Use `ElementServiceUnitTestHelper.ElementServiceUnitTester()` from `TestDomain.Commons.Tests` to wire up ElementService mocks
- Test naming: `MethodName_Scenario_ExpectedResult` (e.g., `GetPmrtData_ValidInput_ReturnsPrograms`)

### Configuration

- Connection strings: stored in User Secrets on AppHost (never in source control)
- Each service has `appsettings.json` with `SourceId`, `Name`, and `ConnectionNames.Cache`
- Gateway URL injected as environment variable `GraphQL__GatewayUrl`

### Logging

Serilog with structured logging to SQL Server (`dbo.SerilogLogs`) and console. Use `ILogger<T>` injection. Include correlation IDs for request tracing.

## Reference docs

Detailed architecture documentation lives in `API/docs/`:
- `GettingStarted.md` — environment setup, running locally
- `BuildingYourFirstMicroservice.md` — step-by-step new service creation
- `DataSourcePatterns.md` — integration patterns by data source type
- `GatewayAndFederation.md` — GraphQL federation, resolvers, cross-service queries
- `CommonLibraries.md` — ElementService API, caching, filters, attributes
- `GraphQLAndREST.md` — API design, versioning, query construction
