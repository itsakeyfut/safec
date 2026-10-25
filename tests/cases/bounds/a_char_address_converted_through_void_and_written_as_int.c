int main(void) {
    char c = 0;
    void *v = &c;
    int *q = v;
    *q = 1;
    return 0;
}
