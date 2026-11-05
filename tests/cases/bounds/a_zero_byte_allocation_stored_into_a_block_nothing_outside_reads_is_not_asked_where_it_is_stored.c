void *malloc(int n);

int main(void) {
    int **m = malloc(8);
    if (m == 0) {
        return 0;
    }
    int *p = malloc(0);
    *m = p;
    return 0;
}
