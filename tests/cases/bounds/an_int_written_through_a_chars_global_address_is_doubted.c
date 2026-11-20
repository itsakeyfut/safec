char c;
int main(void) {
    void *v;
    int *p;
    v = &c;
    p = v;
    *p = 1;
    return 0;
}
