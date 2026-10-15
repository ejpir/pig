package dev.pi.bunembedding;

import android.app.Activity;
import android.os.Bundle;
import android.os.Process;
import android.os.SystemClock;
import android.widget.ScrollView;
import android.widget.TextView;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.FileNotFoundException;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.concurrent.LinkedBlockingQueue;
import org.json.JSONArray;
import org.json.JSONObject;

/** Separate offline test package. No internet permission, accounts or real providers. */
public final class ProbeActivity extends Activity {
  static {System.loadLibrary("bunembeddingpoc");}
  private static native int nativePid();
  private static native int nativeInit();
  private static native byte[] nativeEval(byte[] source);
  private static native int nativePump(int maximumWaitMs);
  private static final LinkedBlockingQueue<Runnable> jobs=new LinkedBlockingQueue<>();
  // JSC/Bun initialization and every call stay on this one permanent 8-MiB-stack worker.
  static {
    Thread worker=new Thread(null,()->{
      for(;;)try{jobs.take().run();}catch(InterruptedException error){return;}
    },"bun-jni-poc",8*1024*1024);
    worker.start();
  }
  private static String evaluate(String source) {
    return new String(nativeEval(source.getBytes(StandardCharsets.UTF_8)),StandardCharsets.UTF_8);
  }
  private File copySdkAsset() throws Exception {
    InputStream input;
    try{input=getAssets().open("durable-probe.bundle.mjs");}catch(FileNotFoundException absent){return null;}
    File directory=new File(getFilesDir(),"embedded-durable");
    if(!directory.isDirectory()&&!directory.mkdirs())throw new IllegalStateException("Cannot create SDK asset directory");
    File target=new File(directory,"durable-probe.bundle.mjs");
    MessageDigest digest=MessageDigest.getInstance("SHA-256");
    try(InputStream source=input;FileOutputStream output=new FileOutputStream(target)) {
      byte[] bytes=new byte[32768];int count;long total=0;
      while((count=source.read(bytes))!=-1) {
        total+=count;if(total>24*1024*1024)throw new IllegalStateException("SDK bundle size limit");
        digest.update(bytes,0,count);output.write(bytes,0,count);
      }
      output.getFD().sync();
    }
    StringBuilder hash=new StringBuilder();for(byte value:digest.digest())hash.append(String.format("%02x",value&255));
    byte[] expected=new byte[128];int count;
    try(InputStream checksum=getAssets().open("durable-probe.sha256")){count=checksum.read(expected);}
    if(count<1||!hash.toString().equals(new String(expected,0,count,StandardCharsets.UTF_8).trim()))
      throw new IllegalStateException("SDK asset checksum mismatch");
    return target;
  }
  private JSONObject durableProbe(File module,String phase) throws Exception {
    if(!phase.equals("create")&&!phase.equals("recover"))throw new IllegalArgumentException("Unknown durable test phase");
    File directory=new File(getNoBackupFilesDir(),"durable-poc");
    if(!directory.isDirectory()&&!directory.mkdirs())throw new IllegalStateException("Cannot create durable state directory");
    JSONObject options=new JSONObject();options.put("root",directory.getAbsolutePath());options.put("phase",phase);
    String uri="file://"+module.getAbsolutePath();
    String source="globalThis.__durableProbe={status:'pending'};void import("+JSONObject.quote(uri)+")"+
      ".then(module=>module.runProbe("+options.toString()+"))"+
      ".then(value=>{globalThis.__durableProbe={status:'done',value}})"+
      ".catch(error=>{globalThis.__durableProbe={status:'failed',error:String(error),stack:String(error?.stack||'').slice(0,6000)}});'started'";
    evaluate(source);
    long deadline=SystemClock.elapsedRealtime()+60000;
    while(SystemClock.elapsedRealtime()<deadline) {
      int status=nativePump(5);
      if(status!=0)throw new IllegalStateException("Bun event pump returned "+status);
      JSONObject state=new JSONObject(evaluate("JSON.stringify(globalThis.__durableProbe)"));
      if(state.getString("status").equals("done"))return state.getJSONObject("value");
      if(state.getString("status").equals("failed"))throw new IllegalStateException(state.toString());
      Thread.sleep(2);
    }
    throw new IllegalStateException("Durable SDK deadline exceeded");
  }
  @Override public void onCreate(Bundle saved) {
    super.onCreate(saved);
    TextView view=new TextView(this);view.setPadding(24,80,24,24);view.setText("Loading linked Bun…");
    ScrollView scroll=new ScrollView(this);scroll.addView(view);setContentView(scroll);
    String phase=getIntent().getStringExtra("durable_phase");if(phase==null)phase="create";
    final String durablePhase=phase;
    jobs.add(()->{
      JSONObject report=new JSONObject();
      try {
        report.put("syntheticOnly",true);report.put("execution","JNI-linked Bun; SDK/tool probe may spawn command children, never a Bun CLI");
        report.put("javaPid",Process.myPid());report.put("nativePid",nativePid());
        int code=nativeInit();report.put("initStatus",code);
        if(code!=0)throw new IllegalStateException("Bun initialization returned "+code);
        JSONArray results=new JSONArray();
        String source="JSON.stringify({bun:Bun.version,buffer:Buffer.from('ok').toString('hex'),pid:process.pid,count:(globalThis.__pocCount=(globalThis.__pocCount||0)+1)})";
        for(int i=0;i<2;i++)results.put(new JSONObject(evaluate(source)));
        report.put("results",results);
        boolean passed=true;
        for(int i=0;i<2;i++) {
          JSONObject result=results.getJSONObject(i);
          passed &= (result.getString("bun").equals("1.4.3") || result.getString("bun").equals("1.4.3-debug")) &&
            result.getString("buffer").equals("6f6b") && result.getInt("pid")==Process.myPid() && result.getInt("count")==i+1;
        }
        File sdk=copySdkAsset();report.put("durableSdkBundled",sdk!=null);
        if(sdk!=null) {
          runOnUiThread(()->view.setText("Bun JNI passed. Running Durable SDK / tools: "+durablePhase+"…"));
          JSONObject result=durableProbe(sdk,durablePhase);report.put("durable",result);passed &= result.optBoolean("passed",false);
        }
        report.put("returnedToJava",true);report.put("passed",passed);
      } catch(Throwable error) {
        try{report.put("passed",false);report.put("error",error.toString());}catch(Exception ignored){}
      }
      try {
        File target=new File(getFilesDir(),"report.json");
        try(FileOutputStream stream=new FileOutputStream(target)){stream.write((report.toString(2)+"\n").getBytes(StandardCharsets.UTF_8));stream.getFD().sync();}
        String result=report.toString(2);runOnUiThread(()->view.setText(result));
      } catch(Exception error) {runOnUiThread(()->view.setText("Report write failed: "+error));}
    });
  }
}
