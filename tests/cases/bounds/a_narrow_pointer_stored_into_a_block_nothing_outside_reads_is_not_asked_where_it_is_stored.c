void *malloc(int n);

int main(void) {
    char c = 0;
    int **m = malloc(8);
    if (m == 0) {
        return 0;
    }
    void *vc = &c;
    int *v = vc;
    *m = v;
    return 0;
}
