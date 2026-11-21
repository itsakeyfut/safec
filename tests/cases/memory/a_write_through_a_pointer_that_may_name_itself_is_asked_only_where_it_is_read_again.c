void put(char **out);
int self_no_read(void) {
    char *p = (char *)&p;
    *p = 1;
    return 0;
}
int self_then_read(void) {
    char *p = (char *)&p;
    *p = 1;
    return p != 0;
}
int after_an_out_parameter(void) {
    char c = 0;
    char *p = &c;
    put(&p);
    if (p) {
        *p = 'x';
    }
    return 0;
}
