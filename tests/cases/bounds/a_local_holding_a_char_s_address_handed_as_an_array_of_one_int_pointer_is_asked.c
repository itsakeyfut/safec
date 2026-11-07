void use(int *(*pi)[1]);
int main(void) {
    char c;
    char *v = &c;
    void *pv = &v;
    use(pv);
    return 0;
}
