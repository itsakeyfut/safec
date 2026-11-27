int g;
int *p = &g;
int a[4];
int *third = &a[2];
int *last = a + 3;
struct S {
    int x, y;
} s;
int *y = &s.y;
int main(void) {
    return 0;
}
