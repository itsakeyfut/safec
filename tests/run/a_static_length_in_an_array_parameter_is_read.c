static int first(int a[static 2]) { return a[0] + a[1]; }
static int last(int a[const 3]) { return a[2]; }
static int both(int a[restrict 3], int b[const static restrict 1]) { return a[1] + b[0]; }
static int rows(int m[const 2][3]) { return m[1][2]; }

int main(void) {
    int a[3];
    int m[2][3];
    a[0] = 1;
    a[1] = 2;
    a[2] = 3;
    m[1][2] = 4;
    return (first(a) == 3 ? 0 : 1) + (last(a) == 3 ? 0 : 1) + (both(a, a) == 3 ? 0 : 1) +
           (rows(m) == 4 ? 0 : 1);
}
