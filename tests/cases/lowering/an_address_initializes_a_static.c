int g;
int *p = &g;
int a[4];
int *third = &a[2];
int *last = a + 3;
int *back = a + 3 - 1;
int *ahead = 3 + a;
int *swapped = &1[a];
char *bytes = (char *)&a[1];
struct S {
    int x, y;
    int v[2];
} s;
int *y = &s.y;
int *member = s.v + 1;
int *element = &s.v[1];
struct S *ps = &s;
int *arrow = &(&s)->y;
int m[2][2];
int *row = m[1];
int *before = a - 1;
int t[3] = {1, 2};
int sparse[100] = {[99] = 1};
struct Named {
    char name[4];
    int k;
} named = {"xy", 3};
int main(void) {
    return 0;
}
