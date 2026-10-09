int x;
int g = x;
struct S { int *p; } s;
int h = *s.p;
int main(void) {
    return 0;
}
