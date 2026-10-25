void *malloc(int n);

int main(void) {
    int *p = malloc(16);
    if (p == 0) {
        return 1;
    }
    int *v = p;
    int **pi = &v;
    v = p + 4;
    **pi = 1;
    return 0;
}
