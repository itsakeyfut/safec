int main(int n) {
    char c;
    int x;
    void *w = &c;
    int *v = &x;
    int *u = w;
    int **pi = &v;
    if (n) {
        pi = &u;
    }
    **pi = 1;
    return 0;
}
