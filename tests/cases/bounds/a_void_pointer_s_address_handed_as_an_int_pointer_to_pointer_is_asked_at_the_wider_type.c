void use(int **pi) {
    if (pi != 0) {
        int *p = *pi;
        if (p != 0) {
            *p = 1;
        }
    }
}

int main(void) {
    char c;
    void *v = &c;
    void *pv = &v;
    use(pv);
    return 0;
}
