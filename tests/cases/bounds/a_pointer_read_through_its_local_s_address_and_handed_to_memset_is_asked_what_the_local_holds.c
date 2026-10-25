void *memset(void *s, int c, int n);

int main(void) {
    char c;
    void *w = &c;
    int *v = w;
    int **pi = &v;
    memset(*pi, 0, 4);
    return 0;
}
