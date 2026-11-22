void *memcpy(void *d, const void *s, unsigned long long n);
char *strcat(const char *d, const char *s);
void free(void *p);
unsigned long long strlen(const char *s);
char *strdup(const char *s);
int measured(void) {
    return (int)strlen("abc");
}
void copied(void) {
    char *d = strdup("abc");
    free(d);
}
int through_a_local(void) {
    const char *p = "abc";
    return (int)strlen(p);
}
int past_its_start(void) {
    return (int)strlen("abc" + 1);
}
int of_a_parameter(const char *q) {
    return q ? (int)strlen(q) : 0;
}
void copied_for_a_length_not_read(char *buf, unsigned long long n) {
    if (buf) {
        memcpy(buf, "abc", n);
    }
}
void appended_to_a_literal_declared_const(void) {
    strcat("ab", "abcdefgh");
}
