void *calloc(int m, int n);

int main(void) {
    int *p = calloc(2, 2);
    int *q = calloc(1, 2);
    if (p == 0) {
        return 1;
    }
    if (q == 0) {
        return 1;
    }
    *p = 1;
    *q = 1;
    return 0;
}
