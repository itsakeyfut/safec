struct B { int n; int d[]; };
union U { struct B b; int x; };
struct Nested { int k; union U u; };
union Fine { union U u; int y; };
