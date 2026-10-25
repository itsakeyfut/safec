void *malloc(int n);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 1;
    }
    int *q = p + 5;
    if (q == 0) {
        return 1;
    }
    *q = 1;
    return 0;
}
