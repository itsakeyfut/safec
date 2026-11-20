int g;
int g = 3;
static int h = 4;
static int twice(int n) {
    return n * 2;
}
int main(void) {
    static int n;
    extern int g;
    n = n + 1;
    return twice(g + h + n);
}
