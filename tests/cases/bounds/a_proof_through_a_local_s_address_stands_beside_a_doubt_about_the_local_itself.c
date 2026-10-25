int main(void) {
    char c;
    void *w = &c;
    int *v = w;
    int **pi = &v;
    *v = **pi;
    return 0;
}
