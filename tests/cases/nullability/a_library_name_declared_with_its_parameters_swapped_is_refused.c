char *strchr(int c, char *s);
int f(void) {
    char *r = strchr(1, 0);
    return r == 0;
}
