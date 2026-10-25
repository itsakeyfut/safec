void put(int *t) {
    if (t) {
        *t = 1;
    }
}

int main(void) {
    char c;
    void *w = &c;
    int *v = w;
    int **pi = &v;
    put(*pi);
    return 0;
}
