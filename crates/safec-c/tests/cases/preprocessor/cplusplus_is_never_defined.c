#ifdef __cplusplus
extern "C" {
#endif
int f(void);
#ifdef __cplusplus
}
#endif
#ifndef __cplusplus
int c_only(void);
#endif

int main(void) { return c_only(); }
